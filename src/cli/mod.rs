use crate::contracts::{Contract, DecodedRunMethodResult, RunMethodResultExt};
use crate::liteclient::{balancer::LiteBalancer, client::LiteClient, rate_limit::RequestRateLimit};
use crate::network_config::ConfigGlobal;
use crate::tl::{AccountId, BlockIdExt, Int256, common::TransactionId3};
use crate::tlb::TlbDeserialize;
use crate::tvm::{Cell, TvmStack, TvmStackEntry, address::Address};
use anyhow::{Context, Result};
use base64::Engine;
use clap::{Parser, Subcommand, ValueEnum};
use num_bigint::BigInt;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Network {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
    PrettyJson,
    Raw,
    Hex,
    Base64,
}

/// tonutils command line interface.
#[derive(Parser, Debug)]
#[command(name = "tonutils")]
#[command(about = "Scriptable TON LiteClient and tooling CLI", long_about = None)]
pub struct Cli {
    /// Network used when downloading the public global config.
    #[arg(long, global = true, default_value = "mainnet")]
    pub network: Network,
    /// Read global config JSON from a file instead of downloading it.
    #[arg(long, global = true)]
    pub config: Option<String>,
    /// Use inline global config JSON instead of downloading it.
    #[arg(long, global = true)]
    pub config_json: Option<String>,
    /// Output format.
    #[arg(long, global = true, default_value = "human")]
    pub output: OutputFormat,
    /// Per-liteserver request-per-second limit.
    #[arg(long, global = true)]
    pub rps: Option<NonZeroU32>,
    /// Total balancer request-per-second limit.
    #[arg(long, global = true)]
    pub global_rps: Option<NonZeroU32>,
    /// Number of liteservers used by high-level balancer commands.
    #[arg(long, global = true, default_value = "3")]
    pub num_servers: usize,
    /// Use one selected liteserver for high-level commands.
    #[arg(long, global = true)]
    pub single: bool,
    /// Liteserver index used with --single and legacy LiteClient commands.
    #[arg(long, global = true, default_value = "0")]
    pub ls_index: usize,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Connect through the default backend and print latest chain and peer status.
    Status,
    /// Fetch a readable account state view.
    Account(HighLevelAccountArgs),
    /// Run a get-method with parsed stack arguments.
    Call(HighLevelCallArgs),
    /// Fetch account transaction history when the current last transaction id is available.
    Transactions(HighLevelTransactionsArgs),
    /// Fetch masterchain or block data.
    Block {
        #[command(subcommand)]
        command: BlockCommand,
    },
    /// Fetch blockchain config proofs.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Advanced/raw LiteServer requests through one LiteClient.
    Liteclient {
        #[command(subcommand)]
        command: LiteClientCommand,
    },
    /// Advanced/raw LiteServer requests through LiteBalancer.
    Balancer {
        #[command(subcommand)]
        command: BalancerCommand,
    },
    /// Advanced smart-contract compatibility helpers over LiteClient.
    Contract {
        #[command(subcommand)]
        command: ContractCommand,
    },
    /// Offline TVM, BoC, and TL-B tooling.
    Tvm {
        #[command(subcommand)]
        command: TvmCommand,
    },
}

#[derive(Parser, Debug)]
pub struct HighLevelAccountArgs {
    /// Account address in raw or friendly form.
    pub address: String,
    /// Block id as wc:shard:seqno:root_hash:file_hash. Defaults to latest masterchain block.
    #[arg(long)]
    pub block: Option<String>,
}

#[derive(Parser, Debug)]
pub struct HighLevelCallArgs {
    /// Account address in raw or friendly form.
    pub address: String,
    /// Method name or numeric method id.
    pub method: String,
    /// Stack argument: int:<decimal>, null, cell:<boc-hex>, or slice:<boc-hex>.
    #[arg(long = "arg")]
    pub args: Vec<String>,
    /// Block id as wc:shard:seqno:root_hash:file_hash. Defaults to latest masterchain block.
    #[arg(long)]
    pub block: Option<String>,
}

#[derive(Parser, Debug)]
pub struct HighLevelTransactionsArgs {
    /// Account address in raw or friendly form.
    pub address: String,
    /// Maximum number of transactions to request.
    #[arg(long, default_value = "10")]
    pub count: u32,
}

#[derive(Subcommand, Debug)]
pub enum BlockCommand {
    /// Print latest masterchain block.
    Latest,
    /// Fetch and decode a block BoC by full block id.
    Get {
        /// Block id as wc:shard:seqno:root_hash:file_hash.
        block: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Fetch all config or selected config parameters.
    Get {
        /// Comma-separated config parameter ids, for example 0,17,34.
        #[arg(long)]
        params: Option<String>,
        /// Block id as wc:shard:seqno:root_hash:file_hash. Defaults to latest masterchain block.
        #[arg(long)]
        block: Option<String>,
        #[command(flatten)]
        flags: ConfigModeFlags,
    },
}

#[derive(Subcommand, Debug)]
pub enum TvmCommand {
    /// Decode and inspect a Bag of Cells.
    Boc {
        #[command(subcommand)]
        command: BocCommand,
    },
    /// Verify checked TL-B schema generation output.
    Schema {
        #[command(subcommand)]
        command: SchemaCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum BocCommand {
    /// Decode a BoC and optionally decode the root as a known TL-B type.
    Decode {
        /// BoC bytes as hex.
        #[arg(long, conflicts_with_all = ["base64", "file", "stdin"])]
        hex: Option<String>,
        /// BoC bytes as base64.
        #[arg(long, conflicts_with_all = ["hex", "file", "stdin"])]
        base64: Option<String>,
        /// Read BoC bytes from a file.
        #[arg(long, conflicts_with_all = ["hex", "base64", "stdin"])]
        file: Option<String>,
        /// Read BoC bytes from stdin.
        #[arg(long, conflicts_with_all = ["hex", "base64", "file"])]
        stdin: bool,
        /// Decode the root cell as a known TL-B type.
        #[arg(long)]
        tlb: Option<KnownTlbType>,
        /// Verify exotic Merkle proof/update child hashes when applicable.
        #[arg(long)]
        verify_proof: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SchemaCommand {
    /// Regenerate the Phase 1 TL-B schema summary and compare it with checked-in output.
    Check,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KnownTlbType {
    Message,
    MessageRelaxed,
    Transaction,
    Account,
    Block,
    Config,
    ShardState,
    Proof,
    MerkleUpdate,
}

#[derive(Subcommand, Debug)]
pub enum LiteClientCommand {
    /// Get masterchain info.
    MasterchainInfo {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
    },
    /// Get liteserver version.
    Version {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
    },
    /// Get liteserver time.
    Time {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
    },
    /// Send an already serialized LiteAPI request.
    RawQuery {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        /// Request bytes as hex.
        #[arg(long, conflicts_with_all = ["base64", "file", "stdin"])]
        hex: Option<String>,
        /// Request bytes as base64.
        #[arg(long, conflicts_with_all = ["hex", "file", "stdin"])]
        base64: Option<String>,
        /// Read request bytes from a file.
        #[arg(long, conflicts_with_all = ["hex", "base64", "stdin"])]
        file: Option<String>,
        /// Read request bytes from stdin.
        #[arg(long, conflicts_with_all = ["hex", "base64", "file"])]
        stdin: bool,
    },
    /// Run a get-method with an empty stack.
    RunGetMethod {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        /// Account address.
        #[arg(short = 'a', long)]
        address: String,
        /// Method name.
        #[arg(short = 'm', long)]
        method: String,
    },
    /// Fetch and decode a block BoC.
    RawGetBlock {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
    },
    /// Fetch and decode a block-header proof BoC.
    RawGetBlockHeader {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[arg(long)]
        with_state_update: bool,
        #[arg(long)]
        with_value_flow: bool,
        #[arg(long)]
        with_extra: bool,
        #[arg(long)]
        with_shard_hashes: bool,
        #[arg(long)]
        with_prev_blk_signatures: bool,
    },
    /// Fetch and decode account state.
    GetAccountStateTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        address: String,
        #[arg(long)]
        block: Option<String>,
    },
    /// Fetch account state and print decoded Account and ShardAccount payloads.
    RawGetAccountState {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        address: String,
        #[arg(long)]
        block: Option<String>,
    },
    /// Fetch a compact account-state view at latest masterchain block.
    GetAccountStateSimple {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        address: String,
    },
    /// Fetch and decode shard-info payloads.
    RawGetShardInfo {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[arg(long)]
        workchain: i32,
        #[arg(long)]
        shard: String,
        #[arg(long)]
        exact: bool,
    },
    /// Fetch and decode all-shards-info payloads.
    RawGetAllShardsInfo {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
    },
    /// Fetch all-shards-info as typed shard block ids.
    GetAllShardsInfoTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
    },
    /// Fetch and decode one transaction.
    GetOneTransactionTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[arg(long)]
        account: String,
        #[arg(long)]
        lt: u64,
    },
    /// Fetch and decode a transaction list.
    RawGetTransactions {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        account: String,
        #[arg(long)]
        lt: u64,
        #[arg(long)]
        hash: String,
        #[arg(long)]
        count: u32,
    },
    /// Fetch and decode extended block transaction list.
    RawGetBlockTransactionsExt {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[arg(long)]
        count: u32,
        #[arg(long)]
        after_account: Option<String>,
        #[arg(long)]
        after_lt: Option<u64>,
        #[arg(long)]
        reverse_order: bool,
        #[arg(long)]
        want_proof: bool,
    },
    /// Run a get-method and decode the returned stack.
    RunGetMethodTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        address: String,
        #[arg(long)]
        block: Option<String>,
        #[arg(
            long,
            conflicts_with = "method_id",
            required_unless_present = "method_id"
        )]
        method: Option<String>,
        #[arg(long, conflicts_with = "method", required_unless_present = "method")]
        method_id: Option<u64>,
    },
    /// Fetch and decode full config proof payload.
    GetConfigAllTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[command(flatten)]
        flags: ConfigModeFlags,
    },
    /// Fetch and decode selected config params.
    GetConfigParamsTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[arg(long)]
        params: String,
        #[command(flatten)]
        flags: ConfigModeFlags,
    },
    /// Fetch library cells.
    GetLibrariesTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        libraries: String,
    },
    /// Fetch library cells with proofs.
    GetLibrariesWithProofTyped {
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        #[arg(long)]
        block: String,
        #[arg(long)]
        libraries: String,
        #[arg(long, default_value = "0")]
        mode: u32,
    },
}

#[derive(Subcommand, Debug)]
pub enum BalancerCommand {
    /// Get masterchain info through several liteservers.
    MasterchainInfo {
        /// Number of liteservers to connect.
        #[arg(short = 'n', long, default_value = "3")]
        num_servers: usize,
    },
    /// Print balancer peer counters after startup.
    Status {
        /// Number of liteservers to connect.
        #[arg(short = 'n', long, default_value = "3")]
        num_servers: usize,
    },
    RawGetBlock(BalancerBlockArgs),
    RawGetBlockHeader(BalancerHeaderArgs),
    GetAccountStateTyped(BalancerAddressBlockArgs),
    RawGetAccountState(BalancerAddressBlockArgs),
    GetAccountStateSimple(BalancerAddressArgs),
    RawGetShardInfo(BalancerShardInfoArgs),
    RawGetAllShardsInfo(BalancerBlockArgs),
    GetAllShardsInfoTyped(BalancerBlockArgs),
    GetOneTransactionTyped(BalancerOneTransactionArgs),
    RawGetTransactions(BalancerTransactionsArgs),
    RawGetBlockTransactionsExt(BalancerBlockTransactionsExtArgs),
    RunGetMethodTyped(BalancerRunGetMethodArgs),
    GetConfigAllTyped(BalancerConfigAllArgs),
    GetConfigParamsTyped(BalancerConfigParamsArgs),
    GetLibrariesTyped(BalancerLibrariesArgs),
    GetLibrariesWithProofTyped(BalancerLibrariesWithProofArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct ConfigModeFlags {
    #[arg(long)]
    with_state_root: bool,
    #[arg(long)]
    with_libraries: bool,
    #[arg(long)]
    with_state_extra_root: bool,
    #[arg(long)]
    with_shard_hashes: bool,
    #[arg(long)]
    with_validator_set: bool,
    #[arg(long)]
    with_special_smc: bool,
    #[arg(long)]
    with_accounts_root: bool,
    #[arg(long)]
    with_prev_blocks: bool,
    #[arg(long)]
    with_workchain_info: bool,
    #[arg(long)]
    with_capabilities: bool,
    #[arg(long)]
    extract_from_key_block: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerBlockArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
}

#[derive(Parser, Debug)]
pub struct BalancerHeaderArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[arg(long)]
    with_state_update: bool,
    #[arg(long)]
    with_value_flow: bool,
    #[arg(long)]
    with_extra: bool,
    #[arg(long)]
    with_shard_hashes: bool,
    #[arg(long)]
    with_prev_blk_signatures: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerAddressBlockArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    address: String,
    #[arg(long)]
    block: Option<String>,
}

#[derive(Parser, Debug)]
pub struct BalancerAddressArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    address: String,
}

#[derive(Parser, Debug)]
pub struct BalancerShardInfoArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[arg(long)]
    workchain: i32,
    #[arg(long)]
    shard: String,
    #[arg(long)]
    exact: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerOneTransactionArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[arg(long)]
    account: String,
    #[arg(long)]
    lt: u64,
}

#[derive(Parser, Debug)]
pub struct BalancerTransactionsArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    account: String,
    #[arg(long)]
    lt: u64,
    #[arg(long)]
    hash: String,
    #[arg(long)]
    count: u32,
}

#[derive(Parser, Debug)]
pub struct BalancerBlockTransactionsExtArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[arg(long)]
    count: u32,
    #[arg(long)]
    after_account: Option<String>,
    #[arg(long)]
    after_lt: Option<u64>,
    #[arg(long)]
    reverse_order: bool,
    #[arg(long)]
    want_proof: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerRunGetMethodArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    address: String,
    #[arg(long)]
    block: Option<String>,
    #[arg(
        long,
        conflicts_with = "method_id",
        required_unless_present = "method_id"
    )]
    method: Option<String>,
    #[arg(long, conflicts_with = "method", required_unless_present = "method")]
    method_id: Option<u64>,
}

#[derive(Parser, Debug)]
pub struct BalancerConfigAllArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[command(flatten)]
    flags: ConfigModeFlags,
}

#[derive(Parser, Debug)]
pub struct BalancerConfigParamsArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[arg(long)]
    params: String,
    #[command(flatten)]
    flags: ConfigModeFlags,
}

#[derive(Parser, Debug)]
pub struct BalancerLibrariesArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    libraries: String,
}

#[derive(Parser, Debug)]
pub struct BalancerLibrariesWithProofArgs {
    #[arg(short = 'n', long, default_value = "3")]
    num_servers: usize,
    #[arg(long)]
    block: String,
    #[arg(long)]
    libraries: String,
    #[arg(long, default_value = "0")]
    mode: u32,
}

#[derive(Subcommand, Debug)]
pub enum ContractCommand {
    /// Fetch account state at the latest masterchain block.
    State {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        /// Account address.
        #[arg(short = 'a', long)]
        address: String,
    },
    /// Run a get-method at the latest masterchain block with an empty stack.
    RunGetMethod {
        /// LiteServer index in the global config.
        #[arg(short = 'l', long, default_value = "0")]
        ls_index: usize,
        /// Account address.
        #[arg(short = 'a', long)]
        address: String,
        /// Method name, converted with the TON CRC16 convention.
        #[arg(
            short = 'm',
            long,
            conflicts_with = "method_id",
            required_unless_present = "method_id"
        )]
        method: Option<String>,
        /// Numeric method id.
        #[arg(long, conflicts_with = "method", required_unless_present = "method")]
        method_id: Option<u64>,
    },
}

#[derive(Debug, Serialize)]
struct BlockIdExtView {
    workchain: i32,
    shard: i64,
    seqno: i32,
    root_hash: String,
    file_hash: String,
}

#[derive(Debug, Serialize)]
struct MasterchainInfoView {
    last: BlockIdExtView,
    state_root_hash: String,
    init_workchain: i32,
    init_root_hash: String,
    init_file_hash: String,
}

#[derive(Debug, Serialize)]
struct VersionView {
    mode: u32,
    version: u32,
    capabilities: u64,
    now: u32,
}

#[derive(Debug, Serialize)]
struct TimeView {
    now: u32,
}

#[derive(Debug, Serialize)]
struct RawBytesView {
    hex: String,
    base64: String,
    len: usize,
}

#[derive(Debug, Serialize)]
struct CellView {
    bits: usize,
    refs: usize,
    exotic: bool,
    level: u8,
    depth: u16,
    hash: String,
}

#[derive(Debug, Serialize)]
struct BocDecodeView {
    raw: RawBytesView,
    root: CellView,
    tlb_type: Option<String>,
    tlb: Option<Value>,
    proof_verified: Option<bool>,
}

#[derive(Debug, Serialize)]
struct SchemaCheckView {
    schema: &'static str,
    constructors: usize,
    generated_matches: bool,
}

#[derive(Debug, Serialize)]
struct RunGetMethodView {
    block: BlockIdExtView,
    shard_block: BlockIdExtView,
    method: Option<String>,
    method_id: u64,
    exit_code: i32,
    shard_proof_len: usize,
    proof_len: usize,
    state_proof_len: usize,
    result: Option<RawBytesView>,
    decoded_stack: Option<TvmStackView>,
    result_decode_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct AccountStateView {
    block: BlockIdExtView,
    shard_block: BlockIdExtView,
    shard_proof_len: usize,
    proof_len: usize,
    state: RawBytesView,
}

#[derive(Debug, Serialize)]
struct TvmStackView {
    entries: Vec<TvmStackEntryView>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TvmStackEntryView {
    Null,
    Int { decimal: String },
    Cell { boc: RawBytesView },
    Slice { boc: RawBytesView },
    Tuple { entries: Vec<TvmStackEntryView> },
    List { entries: Vec<TvmStackEntryView> },
    Unsupported { raw: RawBytesView },
}

#[derive(Debug, Serialize)]
struct BalancerStatusView {
    total_peers: usize,
    alive_peers: usize,
    archival_peers: usize,
}

#[derive(Debug, Serialize)]
struct StatusView {
    network: NetworkView,
    backend: BackendView,
    latest: BlockIdExtView,
    peers: Option<BalancerStatusView>,
}

#[derive(Debug, Serialize)]
struct NetworkView {
    name: &'static str,
}

#[derive(Debug, Serialize)]
struct BackendView {
    mode: &'static str,
    ls_index: Option<usize>,
    num_servers: Option<usize>,
}

#[derive(Debug, Serialize)]
struct BestEffortAccountStateView {
    address: String,
    block: BlockIdExtView,
    shard_block: BlockIdExtView,
    state: String,
    balance: Option<String>,
    last_transaction_lt: Option<u64>,
    last_transaction_hash: Option<String>,
    shard_proof_len: usize,
    proof_len: usize,
    state_len: usize,
    shard_proof_root_count: Option<usize>,
    proof_root_count: Option<usize>,
    shard_proof_root_hash: Option<String>,
    proof_root_hash: Option<String>,
    shard_proof_root_hashes: Vec<String>,
    proof_root_hashes: Vec<String>,
    state_root_hash: Option<String>,
    account: Option<Value>,
    shard_account: Option<Value>,
    decode_errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HighLevelCallView {
    address: String,
    block: BlockIdExtView,
    shard_block: BlockIdExtView,
    method: Option<String>,
    method_id: u64,
    exit_code: i32,
    stack: Option<TvmStackView>,
    decode_errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HighLevelTransactionsView {
    address: String,
    count: u32,
    start_lt: Option<u64>,
    start_hash: Option<String>,
    ids: Vec<BlockIdExtView>,
    transactions: Vec<Value>,
    decode_errors: Vec<String>,
}

fn block_id_ext_view(block: &BlockIdExt) -> BlockIdExtView {
    BlockIdExtView {
        workchain: block.workchain,
        shard: block.shard,
        seqno: block.seqno,
        root_hash: block.root_hash.to_hex(),
        file_hash: block.file_hash.to_hex(),
    }
}

fn raw_bytes_view(bytes: &[u8]) -> RawBytesView {
    RawBytesView {
        hex: hex::encode(bytes),
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        len: bytes.len(),
    }
}

fn cell_view(cell: &Cell) -> CellView {
    CellView {
        bits: cell.bit_len(),
        refs: cell.reference_count(),
        exotic: cell.is_exotic(),
        level: cell.level(),
        depth: cell.depth(),
        hash: hex::encode(cell.hash()),
    }
}

fn stack_view(stack: &TvmStack) -> Result<TvmStackView> {
    Ok(TvmStackView {
        entries: stack
            .entries()
            .iter()
            .map(stack_entry_view)
            .collect::<Result<Vec<_>>>()?,
    })
}

fn bigint_decimal(value: &BigInt) -> String {
    value.to_str_radix(10)
}

fn stack_entry_view(entry: &TvmStackEntry) -> Result<TvmStackEntryView> {
    Ok(match entry {
        TvmStackEntry::Null => TvmStackEntryView::Null,
        TvmStackEntry::Int(value) => TvmStackEntryView::Int {
            decimal: bigint_decimal(value),
        },
        TvmStackEntry::Cell(cell) => TvmStackEntryView::Cell {
            boc: raw_bytes_view(&crate::tvm::serialize_boc(cell, false)?),
        },
        TvmStackEntry::Slice(cell) => TvmStackEntryView::Slice {
            boc: raw_bytes_view(&crate::tvm::serialize_boc(cell, false)?),
        },
        TvmStackEntry::Tuple(entries) => TvmStackEntryView::Tuple {
            entries: entries
                .iter()
                .map(stack_entry_view)
                .collect::<Result<Vec<_>>>()?,
        },
        TvmStackEntry::List(entries) => TvmStackEntryView::List {
            entries: entries
                .iter()
                .map(stack_entry_view)
                .collect::<Result<Vec<_>>>()?,
        },
        TvmStackEntry::Unsupported(bytes) => TvmStackEntryView::Unsupported {
            raw: raw_bytes_view(bytes),
        },
    })
}

fn parse_block_id_ext(value: &str) -> Result<BlockIdExt> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 5 {
        anyhow::bail!("--block must have format wc:shard:seqno:root_hash:file_hash");
    }
    let workchain = parts[0].parse::<i32>().context("invalid block workchain")?;
    let shard = parse_i64_decimal_or_hex(parts[1]).context("invalid block shard")?;
    let seqno = parts[2].parse::<i32>().context("invalid block seqno")?;
    let root_hash = parse_int256(parts[3]).context("invalid block root_hash")?;
    let file_hash = parse_int256(parts[4]).context("invalid block file_hash")?;
    Ok(BlockIdExt {
        workchain,
        shard,
        seqno,
        root_hash,
        file_hash,
    })
}

fn parse_i64_decimal_or_hex(value: &str) -> Result<i64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Ok(u64::from_str_radix(hex, 16)? as i64)
    } else {
        Ok(value.parse::<i64>()?)
    }
}

fn parse_u64_decimal_or_hex(value: &str) -> Result<u64> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Ok(u64::from_str_radix(hex, 16)?)
    } else {
        Ok(value.parse::<u64>()?)
    }
}

fn parse_int256(value: &str) -> Result<Int256> {
    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if hex.len() != 64 {
        anyhow::bail!("hash must be 32 bytes encoded as 64 hex characters");
    }
    Int256::from_hex(hex).context("failed to decode int256 hex")
}

fn parse_account_id(value: &str) -> Result<AccountId> {
    let (workchain, hash) = value
        .split_once(':')
        .context("--account must have format workchain:hash")?;
    if hash.contains(':') {
        anyhow::bail!("--account must have exactly one ':' separator");
    }
    Ok(AccountId {
        workchain: workchain
            .parse::<i32>()
            .context("invalid account workchain")?,
        id: parse_int256(hash)?,
    })
}

fn parse_params(value: &str) -> Result<Vec<i32>> {
    parse_comma_list(value, |item| {
        item.parse::<i32>().context("invalid config param id")
    })
}

fn parse_libraries(value: &str) -> Result<Vec<Int256>> {
    parse_comma_list(value, parse_int256)
}

fn parse_stack_arg(value: &str) -> Result<TvmStackEntry> {
    if value == "null" {
        return Ok(TvmStackEntry::Null);
    }
    let Some((kind, payload)) = value.split_once(':') else {
        anyhow::bail!("stack arg must be null, int:<decimal>, cell:<boc-hex>, or slice:<boc-hex>");
    };
    match kind {
        "int" => Ok(TvmStackEntry::Int(
            BigInt::parse_bytes(payload.as_bytes(), 10).context("invalid decimal stack int")?,
        )),
        "cell" => Ok(TvmStackEntry::Cell(parse_boc_hex_cell(payload)?)),
        "slice" => Ok(TvmStackEntry::Slice(parse_boc_hex_cell(payload)?)),
        _ => anyhow::bail!("unsupported stack arg kind {kind}"),
    }
}

fn parse_stack_args(values: &[String]) -> Result<TvmStack> {
    values
        .iter()
        .map(|value| parse_stack_arg(value))
        .collect::<Result<Vec<_>>>()
        .map(TvmStack::new)
}

fn parse_boc_hex_cell(value: &str) -> Result<std::sync::Arc<Cell>> {
    let bytes = hex::decode(value.trim()).context("failed to decode stack BoC hex")?;
    crate::tvm::deserialize_boc(&bytes).context("failed to decode stack BoC")
}

fn parse_method_ref(value: &str) -> Result<(Option<String>, u64)> {
    match parse_u64_decimal_or_hex(value) {
        Ok(method_id) => Ok((None, method_id)),
        Err(_) => Ok((
            Some(value.to_owned()),
            crate::utils::method_name_to_id(value),
        )),
    }
}

fn parse_comma_list<T>(value: &str, mut parse: impl FnMut(&str) -> Result<T>) -> Result<Vec<T>> {
    if value.trim().is_empty() {
        anyhow::bail!("comma-separated list must not be empty");
    }
    value
        .split(',')
        .map(|item| {
            let item = item.trim();
            if item.is_empty() {
                anyhow::bail!("comma-separated list contains an empty item");
            }
            parse(item)
        })
        .collect()
}

fn parse_after_transaction(
    account: &Option<String>,
    lt: Option<u64>,
) -> Result<Option<TransactionId3>> {
    match (account, lt) {
        (None, None) => Ok(None),
        (Some(account), Some(lt)) => Ok(Some(TransactionId3 {
            account: parse_int256(account)?,
            lt,
        })),
        _ => anyhow::bail!("--after-account and --after-lt must be provided together"),
    }
}

fn latest_or_explicit_block(client_block: Option<&String>, last: BlockIdExt) -> Result<BlockIdExt> {
    match client_block {
        Some(block) => parse_block_id_ext(block),
        None => Ok(last),
    }
}

fn decoded_boc_view(boc: &crate::liteclient::boc::DecodedBoc) -> Value {
    json!({
        "raw": raw_bytes_view(&boc.raw),
        "root": cell_value(&boc.root),
        "root_hash": boc.root_hash_hex(),
    })
}

fn cell_value(cell: &Cell) -> Value {
    json!(cell_view(cell))
}

fn block_value(block: &crate::tlb::Block) -> Value {
    json!({
        "type": "block",
        "global_id": block.global_id,
        "info": cell_value(&block.info),
        "value_flow": cell_value(&block.value_flow),
        "state_update": cell_value(&block.state_update),
        "extra": cell_value(&block.extra),
    })
}

fn account_value(account: &crate::tlb::Account) -> Value {
    match account {
        crate::tlb::Account::None => json!({ "type": "none" }),
        crate::tlb::Account::Full {
            addr,
            storage_stat,
            storage,
        } => json!({
            "type": "full",
            "address": msg_address_int_value(addr),
            "storage_stat": {
                "last_paid": storage_stat.last_paid,
                "due_payment": storage_stat.due_payment.as_ref().map(grams_decimal),
            },
            "storage": {
                "last_trans_lt": storage.last_trans_lt,
                "balance": currency_collection_value(&storage.balance),
                "state": account_state_value(&storage.state),
            }
        }),
    }
}

fn shard_account_value(shard: &crate::tlb::ShardAccount) -> Value {
    json!({
        "account": account_value(&shard.account),
        "last_trans_hash": hex::encode(shard.last_trans_hash),
        "last_trans_lt": shard.last_trans_lt,
    })
}

fn transaction_value(tx: &crate::tlb::Transaction) -> Value {
    json!({
        "account_addr": hex::encode(tx.account_addr),
        "lt": tx.lt,
        "prev_trans_hash": hex::encode(tx.prev_trans_hash),
        "prev_trans_lt": tx.prev_trans_lt,
        "now": tx.now,
        "outmsg_cnt": tx.outmsg_cnt,
        "orig_status": account_status_name(tx.orig_status),
        "end_status": account_status_name(tx.end_status),
        "has_in_msg": tx.in_msg.is_some(),
        "out_msgs_key_bits": tx.out_msgs.key_bits(),
        "total_fees": currency_collection_value(&tx.total_fees),
        "state_update": {
            "old_hash": hex::encode(tx.state_update.old_hash),
            "new_hash": hex::encode(tx.state_update.new_hash),
        },
        "description_type": transaction_description_name(&tx.description),
    })
}

fn simple_account_value(account: &crate::liteclient::boc::SimpleAccount) -> Value {
    json!({
        "block_id": block_id_ext_view(&account.block_id),
        "shard_block_id": block_id_ext_view(&account.shard_block_id),
        "last_transaction_lt": account.last_transaction_lt,
        "last_transaction_hash": account.last_transaction_hash.map(hex::encode),
        "state": simple_account_state_name(&account.state),
        "account": account.account.as_ref().map(account_value),
    })
}

fn msg_address_int_value(addr: &crate::tlb::MsgAddressInt) -> Value {
    match addr {
        crate::tlb::MsgAddressInt::Std { anycast, address } => json!({
            "type": "std",
            "anycast": anycast_value(anycast.as_ref()),
            "workchain": address.workchain,
            "hash": hex::encode(address.hash_part),
            "friendly": format!("{address}"),
        }),
        crate::tlb::MsgAddressInt::Var {
            anycast,
            workchain_id,
            address,
            bit_len,
        } => json!({
            "type": "var",
            "anycast": anycast_value(anycast.as_ref()),
            "workchain": workchain_id,
            "address": hex::encode(address),
            "bit_len": bit_len,
        }),
    }
}

fn anycast_value(anycast: Option<&crate::tlb::Anycast>) -> Value {
    match anycast {
        Some(anycast) => json!({
            "depth": anycast.depth,
            "rewrite_pfx": hex::encode(&anycast.rewrite_pfx),
        }),
        None => Value::Null,
    }
}

fn account_state_value(state: &crate::tlb::AccountState) -> Value {
    match state {
        crate::tlb::AccountState::Uninit => json!({ "type": "uninit" }),
        crate::tlb::AccountState::Frozen { state_hash } => {
            json!({ "type": "frozen", "state_hash": hex::encode(state_hash) })
        }
        crate::tlb::AccountState::Active { state_init } => json!({
            "type": "active",
            "has_code": state_init.code.is_some(),
            "has_data": state_init.data.is_some(),
            "has_library": state_init.library.is_some(),
        }),
    }
}

fn currency_collection_value(value: &crate::tlb::CurrencyCollection) -> Value {
    json!({
        "grams": grams_decimal(&value.grams),
        "other": { "key_bits": value.other.key_bits() },
    })
}

fn grams_decimal(value: &crate::tlb::Grams) -> String {
    value.0.to_str_radix(10)
}

fn account_status_name(status: crate::tlb::AccountStatus) -> &'static str {
    match status {
        crate::tlb::AccountStatus::Uninit => "uninit",
        crate::tlb::AccountStatus::Frozen => "frozen",
        crate::tlb::AccountStatus::Active => "active",
        crate::tlb::AccountStatus::Nonexist => "nonexist",
    }
}

fn simple_account_state_name(state: &crate::liteclient::boc::SimpleAccountState) -> &'static str {
    match state {
        crate::liteclient::boc::SimpleAccountState::None => "none",
        crate::liteclient::boc::SimpleAccountState::Uninit => "uninit",
        crate::liteclient::boc::SimpleAccountState::Frozen => "frozen",
        crate::liteclient::boc::SimpleAccountState::Active => "active",
    }
}

fn transaction_description_name(description: &crate::tlb::TransactionDescr) -> &'static str {
    match description {
        crate::tlb::TransactionDescr::Ordinary { .. } => "ordinary",
        crate::tlb::TransactionDescr::Storage { .. } => "storage",
        crate::tlb::TransactionDescr::TickTock { .. } => "tick_tock",
        crate::tlb::TransactionDescr::SplitPrepare { .. } => "split_prepare",
        crate::tlb::TransactionDescr::SplitInstall { .. } => "split_install",
        crate::tlb::TransactionDescr::MergePrepare { .. } => "merge_prepare",
        crate::tlb::TransactionDescr::MergeInstall { .. } => "merge_install",
    }
}

fn decoded_block_data_value(decoded: &crate::liteclient::boc::DecodedBlockData) -> Value {
    json!({
        "id": block_id_ext_view(&decoded.raw.id),
        "data": {
            "boc": decoded_boc_view(&decoded.data.boc),
            "block": block_value(&decoded.data.block),
        }
    })
}

fn decoded_block_header_value(decoded: &crate::liteclient::boc::DecodedBlockHeader) -> Value {
    json!({
        "id": block_id_ext_view(&decoded.raw.id),
        "mode": decoded.raw.mode,
        "header_proof": decoded_boc_view(&decoded.header_proof),
    })
}

fn decoded_shard_info_value(decoded: &crate::liteclient::boc::DecodedShardInfo) -> Value {
    json!({
        "id": block_id_ext_view(&decoded.raw.id),
        "shardblk": block_id_ext_view(&decoded.raw.shardblk),
        "shard_proof": decoded.shard_proof.as_ref().map(decoded_boc_view),
        "shard_descr": decoded_boc_view(&decoded.shard_descr.boc),
    })
}

fn decoded_all_shards_info_value(decoded: &crate::liteclient::boc::DecodedAllShardsInfo) -> Value {
    json!({
        "id": block_id_ext_view(&decoded.raw.id),
        "proof": decoded.proof.as_ref().map(decoded_boc_view),
        "data": decoded_boc_view(&decoded.data),
    })
}

fn decoded_config_info_value(decoded: &crate::liteclient::boc::DecodedConfigInfo) -> Value {
    json!({
        "mode": decoded.raw.mode,
        "id": block_id_ext_view(&decoded.raw.id),
        "state_proof": decoded.state_proof.as_ref().map(decoded_boc_view),
        "config_proof": decoded.config_proof.as_ref().map(|config| json!({
            "boc": decoded_boc_view(&config.boc),
            "config": config_params_value(&config.config),
        })),
    })
}

fn config_params_value(config: &crate::tlb::ConfigParams) -> Value {
    json!({
        "config_addr": hex::encode(config.config_addr),
        "config": cell_value(&config.config),
    })
}

fn shard_state_value(state: &crate::tlb::ShardState) -> Value {
    match state {
        crate::tlb::ShardState::Unsplit { payload } => json!({
            "type": "unsplit",
            "payload": cell_value(payload),
        }),
        crate::tlb::ShardState::Split { left, right } => json!({
            "type": "split",
            "left": cell_value(left),
            "right": cell_value(right),
        }),
    }
}

fn libraries_value(
    libraries: &std::collections::HashMap<Int256, Option<std::sync::Arc<Cell>>>,
) -> Value {
    let mut stable = BTreeMap::new();
    for (hash, cell) in libraries {
        stable.insert(
            hash.to_hex(),
            cell.as_ref()
                .map(|cell| cell_value(cell))
                .unwrap_or(Value::Null),
        );
    }
    json!(stable)
}

fn decoded_libraries_with_proof_value(
    decoded: &crate::liteclient::boc::DecodedLibrariesWithProof,
) -> Value {
    json!({
        "mode": decoded.raw.mode,
        "id": block_id_ext_view(&decoded.raw.id),
        "libraries": libraries_value(&decoded.libraries),
        "state_proof": decoded.state_proof.as_ref().map(decoded_boc_view),
        "data_proof": decoded.data_proof.as_ref().map(decoded_boc_view),
    })
}

fn transactions_value(transactions: &[crate::tlb::Transaction]) -> Value {
    json!(
        transactions
            .iter()
            .map(transaction_value)
            .collect::<Vec<_>>()
    )
}

fn print_block_human(prefix: &str, block: &BlockIdExtView) {
    println!("{prefix}_workchain: {}", block.workchain);
    println!("{prefix}_shard: {}", block.shard);
    println!("{prefix}_seqno: {}", block.seqno);
    println!("{prefix}_root_hash: {}", block.root_hash);
    println!("{prefix}_file_hash: {}", block.file_hash);
}

fn stack_entry_human(entry: &TvmStackEntryView) -> String {
    match entry {
        TvmStackEntryView::Null => "null".to_owned(),
        TvmStackEntryView::Int { decimal } => format!("int:{decimal}"),
        TvmStackEntryView::Cell { boc } => format!("cell len={} hash_unavailable", boc.len),
        TvmStackEntryView::Slice { boc } => format!("slice len={} hash_unavailable", boc.len),
        TvmStackEntryView::Tuple { entries } => format!("tuple len={}", entries.len()),
        TvmStackEntryView::List { entries } => format!("list len={}", entries.len()),
        TvmStackEntryView::Unsupported { raw } => format!("unsupported len={}", raw.len),
    }
}

async fn get_config_all_client(
    client: &mut LiteClient,
    block: BlockIdExt,
    flags: &ConfigModeFlags,
) -> std::result::Result<
    crate::liteclient::boc::DecodedConfigInfo,
    crate::liteclient::types::LiteError,
> {
    client
        .get_config_all_typed(
            block,
            flags.with_state_root,
            flags.with_libraries,
            flags.with_state_extra_root,
            flags.with_shard_hashes,
            flags.with_validator_set,
            flags.with_special_smc,
            flags.with_accounts_root,
            flags.with_prev_blocks,
            flags.with_workchain_info,
            flags.with_capabilities,
            flags.extract_from_key_block,
        )
        .await
}

async fn get_config_params_client(
    client: &mut LiteClient,
    block: BlockIdExt,
    params: Vec<i32>,
    flags: &ConfigModeFlags,
) -> std::result::Result<
    crate::liteclient::boc::DecodedConfigInfo,
    crate::liteclient::types::LiteError,
> {
    client
        .get_config_params_typed(
            block,
            params,
            flags.with_state_root,
            flags.with_libraries,
            flags.with_state_extra_root,
            flags.with_shard_hashes,
            flags.with_validator_set,
            flags.with_special_smc,
            flags.with_accounts_root,
            flags.with_prev_blocks,
            flags.with_workchain_info,
            flags.with_capabilities,
            flags.extract_from_key_block,
        )
        .await
}

async fn get_config_all_balancer(
    balancer: &mut LiteBalancer,
    block: BlockIdExt,
    flags: &ConfigModeFlags,
) -> std::result::Result<
    crate::liteclient::boc::DecodedConfigInfo,
    crate::liteclient::balancer::BalancerError,
> {
    balancer
        .get_config_all_typed(
            block,
            flags.with_state_root,
            flags.with_libraries,
            flags.with_state_extra_root,
            flags.with_shard_hashes,
            flags.with_validator_set,
            flags.with_special_smc,
            flags.with_accounts_root,
            flags.with_prev_blocks,
            flags.with_workchain_info,
            flags.with_capabilities,
            flags.extract_from_key_block,
        )
        .await
}

async fn get_config_params_balancer(
    balancer: &mut LiteBalancer,
    block: BlockIdExt,
    params: Vec<i32>,
    flags: &ConfigModeFlags,
) -> std::result::Result<
    crate::liteclient::boc::DecodedConfigInfo,
    crate::liteclient::balancer::BalancerError,
> {
    balancer
        .get_config_params_typed(
            block,
            params,
            flags.with_state_root,
            flags.with_libraries,
            flags.with_state_extra_root,
            flags.with_shard_hashes,
            flags.with_validator_set,
            flags.with_special_smc,
            flags.with_accounts_root,
            flags.with_prev_blocks,
            flags.with_workchain_info,
            flags.with_capabilities,
            flags.extract_from_key_block,
        )
        .await
}

fn account_state_view(state: crate::tl::response::AccountState) -> AccountStateView {
    AccountStateView {
        block: block_id_ext_view(&state.id),
        shard_block: block_id_ext_view(&state.shardblk),
        shard_proof_len: state.shard_proof.len(),
        proof_len: state.proof.len(),
        state: raw_bytes_view(&state.state),
    }
}

fn run_get_method_view(
    result: crate::tl::response::RunMethodResult,
    method: Option<String>,
    method_id: u64,
) -> Result<RunGetMethodView> {
    let (decoded_stack, result_decode_error) = match result.result_stack_lossless() {
        DecodedRunMethodResult::Missing => (None, None),
        DecodedRunMethodResult::Decoded(stack) => (Some(stack_view(&stack)?), None),
        DecodedRunMethodResult::Undecodable { error, .. } => (None, Some(error)),
    };

    Ok(RunGetMethodView {
        block: block_id_ext_view(&result.id),
        shard_block: block_id_ext_view(&result.shardblk),
        method,
        method_id,
        exit_code: result.exit_code,
        shard_proof_len: result.shard_proof.as_ref().map_or(0, Vec::len),
        proof_len: result.proof.as_ref().map_or(0, Vec::len),
        state_proof_len: result.state_proof.as_ref().map_or(0, Vec::len),
        result: result.raw_result_boc().map(raw_bytes_view),
        decoded_stack,
        result_decode_error,
    })
}

fn masterchain_info_view(info: crate::tl::response::MasterchainInfo) -> MasterchainInfoView {
    MasterchainInfoView {
        last: block_id_ext_view(&info.last),
        state_root_hash: info.state_root_hash.to_hex(),
        init_workchain: info.init.workchain,
        init_root_hash: info.init.root_hash.to_hex(),
        init_file_hash: info.init.file_hash.to_hex(),
    }
}

fn network_name(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
    }
}

enum HighLevelBackend {
    Single(LiteClient),
    Balanced(LiteBalancer),
}

impl HighLevelBackend {
    fn view(&self, cli: &Cli) -> BackendView {
        match self {
            HighLevelBackend::Single(_) => BackendView {
                mode: "single",
                ls_index: Some(cli.ls_index),
                num_servers: None,
            },
            HighLevelBackend::Balanced(_) => BackendView {
                mode: "balancer",
                ls_index: None,
                num_servers: Some(cli.num_servers),
            },
        }
    }

    async fn close(self) -> Result<()> {
        match self {
            HighLevelBackend::Single(_) => Ok(()),
            HighLevelBackend::Balanced(mut balancer) => {
                balancer.close_all().await?;
                Ok(())
            }
        }
    }

    async fn get_masterchain_info(&mut self) -> Result<crate::tl::response::MasterchainInfo> {
        match self {
            HighLevelBackend::Single(client) => Ok(client.get_masterchain_info().await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer.get_masterchain_info().await?),
        }
    }

    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<crate::tl::response::AccountState> {
        match self {
            HighLevelBackend::Single(client) => {
                Ok(client.get_account_state(block, account).await?)
            }
            HighLevelBackend::Balanced(balancer) => {
                Ok(balancer.get_account_state(block, account).await?)
            }
        }
    }

    async fn raw_get_block_data(
        &mut self,
        block: BlockIdExt,
    ) -> Result<crate::liteclient::boc::DecodedBlockData> {
        match self {
            HighLevelBackend::Single(client) => Ok(client.raw_get_block_data(block).await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer.raw_get_block_data(block).await?),
        }
    }

    async fn run_get_method(
        &mut self,
        block: BlockIdExt,
        address: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<crate::tl::response::RunMethodResult> {
        match self {
            HighLevelBackend::Single(client) => Ok(client
                .run_get_method(0, block, address, method_id, stack)
                .await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer
                .run_get_method(0, block, address, method_id, stack)
                .await?),
        }
    }

    async fn raw_get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<(Vec<crate::tlb::Transaction>, Vec<BlockIdExt>)> {
        match self {
            HighLevelBackend::Single(client) => Ok(client
                .raw_get_transactions(count, account, lt, hash)
                .await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer
                .raw_get_transactions(count, account, lt, hash)
                .await?),
        }
    }

    async fn get_config_all_typed(
        &mut self,
        block: BlockIdExt,
        flags: &ConfigModeFlags,
    ) -> Result<crate::liteclient::boc::DecodedConfigInfo> {
        match self {
            HighLevelBackend::Single(client) => {
                Ok(get_config_all_client(client, block, flags).await?)
            }
            HighLevelBackend::Balanced(balancer) => {
                Ok(get_config_all_balancer(balancer, block, flags).await?)
            }
        }
    }

    async fn get_config_params_typed(
        &mut self,
        block: BlockIdExt,
        params: Vec<i32>,
        flags: &ConfigModeFlags,
    ) -> Result<crate::liteclient::boc::DecodedConfigInfo> {
        match self {
            HighLevelBackend::Single(client) => {
                Ok(get_config_params_client(client, block, params, flags).await?)
            }
            HighLevelBackend::Balanced(balancer) => {
                Ok(get_config_params_balancer(balancer, block, params, flags).await?)
            }
        }
    }
}

fn best_effort_account_state_view(
    address: &str,
    raw: crate::tl::response::AccountState,
) -> BestEffortAccountStateView {
    let mut decode_errors = Vec::new();
    let (shard_proof_root_count, shard_proof_root_hashes) =
        decode_root_hashes(&raw.shard_proof, "shard_proof", &mut decode_errors);
    let (proof_root_count, proof_root_hashes) =
        decode_root_hashes(&raw.proof, "proof", &mut decode_errors);
    let shard_proof_root_hash = shard_proof_root_hashes.first().cloned();
    let proof_root_hash = proof_root_hashes.first().cloned();
    let state_root_hash = decode_root_hash(&raw.state, "state", &mut decode_errors);

    let account = if raw.state.is_empty() {
        None
    } else {
        match crate::liteclient::boc::decode_account_state_boc(&raw.state) {
            Ok(value) => Some(value.account),
            Err(error) => {
                decode_errors.push(format!("state TL-B decode failed: {error}"));
                None
            }
        }
    };
    let (state, balance) = account_summary(account.as_ref());
    let last_transaction_lt = account.as_ref().and_then(|account| match account {
        crate::tlb::Account::None => None,
        crate::tlb::Account::Full { storage, .. } => Some(storage.last_trans_lt),
    });

    BestEffortAccountStateView {
        address: address.to_owned(),
        block: block_id_ext_view(&raw.id),
        shard_block: block_id_ext_view(&raw.shardblk),
        state,
        balance,
        last_transaction_lt,
        last_transaction_hash: None,
        shard_proof_len: raw.shard_proof.len(),
        proof_len: raw.proof.len(),
        state_len: raw.state.len(),
        shard_proof_root_count,
        proof_root_count,
        shard_proof_root_hash,
        proof_root_hash,
        shard_proof_root_hashes,
        proof_root_hashes,
        state_root_hash,
        account: account.as_ref().map(account_value),
        shard_account: None,
        decode_errors,
    }
}

fn decode_root_hashes(
    raw: &[u8],
    label: &str,
    errors: &mut Vec<String>,
) -> (Option<usize>, Vec<String>) {
    if raw.is_empty() {
        return (None, Vec::new());
    }
    match crate::tvm::inspect_boc(raw) {
        Ok(inspection) => (Some(inspection.root_count()), inspection.root_hashes_hex()),
        Err(error) => {
            errors.push(format!("{label} BoC decode failed: {error:#}"));
            (None, Vec::new())
        }
    }
}

fn decode_root_hash(raw: &[u8], label: &str, errors: &mut Vec<String>) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    match crate::liteclient::boc::DecodedBoc::decode(raw) {
        Ok(decoded) => Some(decoded.root_hash_hex()),
        Err(error) => {
            errors.push(format!("{label} BoC decode failed: {error:#}"));
            None
        }
    }
}

fn account_summary(account: Option<&crate::tlb::Account>) -> (String, Option<String>) {
    match account {
        None | Some(crate::tlb::Account::None) => ("none".to_owned(), None),
        Some(crate::tlb::Account::Full { storage, .. }) => (
            simple_account_state_name(&match &storage.state {
                crate::tlb::AccountState::Uninit => {
                    crate::liteclient::boc::SimpleAccountState::Uninit
                }
                crate::tlb::AccountState::Frozen { .. } => {
                    crate::liteclient::boc::SimpleAccountState::Frozen
                }
                crate::tlb::AccountState::Active { .. } => {
                    crate::liteclient::boc::SimpleAccountState::Active
                }
            })
            .to_owned(),
            Some(grams_decimal(&storage.balance.grams)),
        ),
    }
}

async fn download_config(network: Network) -> Result<String> {
    let url = match network {
        Network::Mainnet => "https://ton.org/global.config.json",
        Network::Testnet => "https://ton.org/testnet-global.config.json",
    };
    let mut response = ureq::get(url)
        .call()
        .map_err(|e| anyhow::anyhow!("failed to fetch config from {url}: {e:?}"))?;
    if response.status() != 200 {
        anyhow::bail!("config URL {url} returned HTTP {}", response.status());
    }
    Ok(response.body_mut().read_to_string()?)
}

impl Cli {
    pub fn parse_args() -> Self {
        Cli::parse()
    }

    async fn load_config(&self) -> Result<ConfigGlobal> {
        let config_json = match (&self.config_json, &self.config) {
            (Some(json), None) => json.clone(),
            (None, Some(path)) => fs::read_to_string(path)
                .with_context(|| format!("failed to read config file {path}"))?,
            (None, None) => download_config(self.network).await?,
            (Some(_), Some(_)) => {
                anyhow::bail!("--config and --config-json are mutually exclusive")
            }
        };
        ConfigGlobal::from_str(&config_json).context("failed to parse TON global config")
    }

    pub async fn create_client(&self, ls_index: usize) -> Result<LiteClient> {
        let config = self.load_config().await?;
        let mut client = LiteClient::connect_config(&config, ls_index)
            .await
            .map_err(anyhow::Error::from)?;
        if let Some(rps) = self.rps {
            client.set_rate_limit(RequestRateLimit::per_second(rps.get())?);
        }
        Ok(client)
    }

    async fn create_balancer(&self, num_servers: usize) -> Result<LiteBalancer> {
        let config = self.load_config().await?;
        let mut clients = Vec::new();
        let limit = num_servers.min(config.liteservers.len());

        for index in 0..limit {
            match LiteClient::connect_liteserver(&config.liteservers[index]).await {
                Ok(mut client) => {
                    if let Some(rps) = self.rps {
                        client.set_rate_limit(RequestRateLimit::per_second(rps.get())?);
                    }
                    clients.push(client);
                }
                Err(error) => {
                    eprintln!(
                        "failed to connect liteserver #{index} ({}): {error}",
                        config.liteservers[index].socket_addr()
                    );
                }
            }
        }

        if clients.is_empty() {
            anyhow::bail!("failed to connect to any liteserver");
        }

        let mut balancer = LiteBalancer::new(clients, Duration::from_secs(10));
        if let Some(rps) = self.global_rps {
            balancer = balancer.with_global_rate_limit(RequestRateLimit::per_second(rps.get())?);
        }
        balancer.start_up().await?;
        Ok(balancer)
    }

    async fn create_high_level_backend(&self) -> Result<HighLevelBackend> {
        if self.single {
            Ok(HighLevelBackend::Single(
                self.create_client(self.ls_index).await?,
            ))
        } else {
            Ok(HighLevelBackend::Balanced(
                self.create_balancer(self.num_servers).await?,
            ))
        }
    }

    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            Commands::Status => self.execute_status().await,
            Commands::Account(args) => self.execute_account(args).await,
            Commands::Call(args) => self.execute_call(args).await,
            Commands::Transactions(args) => self.execute_transactions(args).await,
            Commands::Block { command } => self.execute_block(command).await,
            Commands::Config { command } => self.execute_config(command).await,
            Commands::Liteclient { command } => self.execute_liteclient(command).await,
            Commands::Balancer { command } => self.execute_balancer(command).await,
            Commands::Contract { command } => self.execute_contract(command).await,
            Commands::Tvm { command } => self.execute_tvm(command).await,
        }
    }

    async fn execute_status(&self) -> Result<()> {
        let mut backend = self.create_high_level_backend().await?;
        let backend_view = backend.view(self);
        let info = backend
            .get_masterchain_info()
            .await
            .context("status: failed to get latest masterchain block")?;
        let peers = match &backend {
            HighLevelBackend::Single(_) => None,
            HighLevelBackend::Balanced(balancer) => Some(BalancerStatusView {
                total_peers: balancer.peers_num(),
                alive_peers: balancer.alive_peers_num().await,
                archival_peers: balancer.archival_peers_num().await,
            }),
        };
        backend.close().await?;
        self.print_status(&StatusView {
            network: NetworkView {
                name: network_name(self.network),
            },
            backend: backend_view,
            latest: block_id_ext_view(&info.last),
            peers,
        })
    }

    async fn execute_account(&self, args: &HighLevelAccountArgs) -> Result<()> {
        let mut backend = self.create_high_level_backend().await?;
        let address = Address::from_str(&args.address).context("account: invalid address")?;
        let latest = backend
            .get_masterchain_info()
            .await
            .with_context(|| format!("account {}: failed to get latest block", args.address))?
            .last;
        let block = latest_or_explicit_block(args.block.as_ref(), latest)?;
        let raw = backend
            .get_account_state(block, address.to_account_id())
            .await
            .with_context(|| {
                format!(
                    "account {}: failed to fetch account state using {} backend",
                    args.address,
                    if self.single { "single" } else { "balancer" }
                )
            })?;
        backend.close().await?;
        self.print_account(&best_effort_account_state_view(&args.address, raw))
    }

    async fn execute_call(&self, args: &HighLevelCallArgs) -> Result<()> {
        let mut backend = self.create_high_level_backend().await?;
        let address = Address::from_str(&args.address).context("call: invalid address")?;
        let latest = backend
            .get_masterchain_info()
            .await
            .with_context(|| format!("call {}: failed to get latest block", args.address))?
            .last;
        let block = latest_or_explicit_block(args.block.as_ref(), latest)?;
        let (method, method_id) = parse_method_ref(&args.method)?;
        let stack = parse_stack_args(&args.args)?;
        let result = backend
            .run_get_method(block, address, method_id, stack)
            .await
            .with_context(|| {
                format!(
                    "call {} {}: failed using {} backend",
                    args.address,
                    args.method,
                    if self.single { "single" } else { "balancer" }
                )
            })?;
        backend.close().await?;
        let (stack, decode_errors) = match result.result_stack_lossless() {
            DecodedRunMethodResult::Missing => (None, Vec::new()),
            DecodedRunMethodResult::Decoded(stack) => (Some(stack_view(&stack)?), Vec::new()),
            DecodedRunMethodResult::Undecodable { error, .. } => (None, vec![error]),
        };
        self.print_call(&HighLevelCallView {
            address: args.address.clone(),
            block: block_id_ext_view(&result.id),
            shard_block: block_id_ext_view(&result.shardblk),
            method,
            method_id,
            exit_code: result.exit_code,
            stack,
            decode_errors,
        })
    }

    async fn execute_transactions(&self, args: &HighLevelTransactionsArgs) -> Result<()> {
        let mut backend = self.create_high_level_backend().await?;
        let address = Address::from_str(&args.address).context("transactions: invalid address")?;
        let latest = backend
            .get_masterchain_info()
            .await
            .with_context(|| format!("transactions {}: failed to get latest block", args.address))?
            .last;
        let raw_account = backend
            .get_account_state(latest, address.to_account_id())
            .await
            .with_context(|| {
                format!(
                    "transactions {}: failed to fetch account state",
                    args.address
                )
            })?;
        let mut account_view = best_effort_account_state_view(&args.address, raw_account);
        let Some(lt) = account_view.last_transaction_lt else {
            backend.close().await?;
            return self.print_transactions(&HighLevelTransactionsView {
                address: args.address.clone(),
                count: args.count,
                start_lt: None,
                start_hash: None,
                ids: Vec::new(),
                transactions: Vec::new(),
                decode_errors: account_view.decode_errors,
            });
        };
        if account_view.state == "none" || lt == 0 {
            backend.close().await?;
            return self.print_transactions(&HighLevelTransactionsView {
                address: args.address.clone(),
                count: args.count,
                start_lt: Some(lt),
                start_hash: account_view.last_transaction_hash,
                ids: Vec::new(),
                transactions: Vec::new(),
                decode_errors: account_view.decode_errors,
            });
        }
        let hash = account_view
            .last_transaction_hash
            .as_deref()
            .map(parse_int256)
            .transpose()?;
        let Some(hash) = hash else {
            account_view.decode_errors.push(
                "transaction history requires last transaction hash from a verified ShardAccounts proof path; extraction is not implemented yet".to_owned(),
            );
            backend.close().await?;
            return self.print_transactions(&HighLevelTransactionsView {
                address: args.address.clone(),
                count: args.count,
                start_lt: Some(lt),
                start_hash: None,
                ids: Vec::new(),
                transactions: Vec::new(),
                decode_errors: account_view.decode_errors,
            });
        };
        let (transactions, ids) = backend
            .raw_get_transactions(args.count, address.to_account_id(), lt, hash)
            .await
            .with_context(|| format!("transactions {}: failed to fetch history", args.address))?;
        backend.close().await?;
        self.print_transactions(&HighLevelTransactionsView {
            address: args.address.clone(),
            count: args.count,
            start_lt: Some(lt),
            start_hash: account_view.last_transaction_hash,
            ids: ids.iter().map(block_id_ext_view).collect(),
            transactions: transactions.iter().map(transaction_value).collect(),
            decode_errors: account_view.decode_errors,
        })
    }

    async fn execute_block(&self, command: &BlockCommand) -> Result<()> {
        let mut backend = self.create_high_level_backend().await?;
        match command {
            BlockCommand::Latest => {
                let info = backend
                    .get_masterchain_info()
                    .await
                    .context("block latest: failed to get masterchain info")?;
                backend.close().await?;
                self.print_structured(&masterchain_info_view(info))
            }
            BlockCommand::Get { block } => {
                let decoded = backend
                    .raw_get_block_data(parse_block_id_ext(block)?)
                    .await
                    .with_context(|| format!("block get {block}: failed to fetch block"))?;
                backend.close().await?;
                self.print_structured(&decoded_block_data_value(&decoded))
            }
        }
    }

    async fn execute_config(&self, command: &ConfigCommand) -> Result<()> {
        match command {
            ConfigCommand::Get {
                params,
                block,
                flags,
            } => {
                let mut backend = self.create_high_level_backend().await?;
                let latest = backend
                    .get_masterchain_info()
                    .await
                    .context("config get: failed to get latest block")?
                    .last;
                let block = latest_or_explicit_block(block.as_ref(), latest)?;
                let decoded = if let Some(params) = params {
                    backend
                        .get_config_params_typed(block, parse_params(params)?, flags)
                        .await
                        .context("config get: failed to fetch selected config params")?
                } else {
                    backend
                        .get_config_all_typed(block, flags)
                        .await
                        .context("config get: failed to fetch full config")?
                };
                backend.close().await?;
                self.print_structured(&decoded_config_info_value(&decoded))
            }
        }
    }

    async fn execute_tvm(&self, command: &TvmCommand) -> Result<()> {
        match command {
            TvmCommand::Boc { command } => self.execute_boc(command),
            TvmCommand::Schema { command } => self.execute_schema(command),
        }
    }

    fn execute_schema(&self, command: &SchemaCommand) -> Result<()> {
        match command {
            SchemaCommand::Check => {
                let generated = crate::tlb::schema::generate_block_phase1()?;
                let constructors =
                    crate::tlb::schema::parse_schema(crate::tlb::schema::BLOCK_PHASE1_TLB)?;
                let view = SchemaCheckView {
                    schema: "block_phase1.tlb",
                    constructors: constructors.len(),
                    generated_matches: generated == crate::tlb::schema::BLOCK_PHASE1_GENERATED,
                };
                if !view.generated_matches {
                    anyhow::bail!("checked-in TL-B generated output is stale");
                }
                self.print_structured(&view)
            }
        }
    }

    fn execute_boc(&self, command: &BocCommand) -> Result<()> {
        match command {
            BocCommand::Decode {
                hex,
                base64,
                file,
                stdin,
                tlb,
                verify_proof,
            } => {
                let raw = read_raw_input(hex, base64, file, *stdin)?;
                let root = crate::tvm::deserialize_boc(&raw).context("failed to decode BoC")?;
                let (tlb_type, tlb, proof_verified) =
                    decode_known_tlb(root.clone(), *tlb, *verify_proof)?;
                self.print_structured(&BocDecodeView {
                    raw: raw_bytes_view(&raw),
                    root: cell_view(&root),
                    tlb_type,
                    tlb,
                    proof_verified,
                })
            }
        }
    }

    async fn execute_liteclient(&self, command: &LiteClientCommand) -> Result<()> {
        match command {
            LiteClientCommand::MasterchainInfo { ls_index } => {
                let mut client = self.create_client(*ls_index).await?;
                let info = client.get_masterchain_info().await?;
                self.print_structured(&masterchain_info_view(info))
            }
            LiteClientCommand::Version { ls_index } => {
                let mut client = self.create_client(*ls_index).await?;
                let version = client.get_version().await?;
                self.print_structured(&VersionView {
                    mode: version.mode,
                    version: version.version,
                    capabilities: version.capabilities,
                    now: version.now,
                })
            }
            LiteClientCommand::Time { ls_index } => {
                let mut client = self.create_client(*ls_index).await?;
                let now = client.get_time().await?;
                self.print_structured(&TimeView { now })
            }
            LiteClientCommand::RawQuery {
                ls_index,
                hex,
                base64,
                file,
                stdin,
            } => {
                let request = read_raw_input(hex, base64, file, *stdin)?;
                let mut client = self.create_client(*ls_index).await?;
                let response = client.query_raw(request).await?;
                self.print_bytes(&response)
            }
            LiteClientCommand::RunGetMethod {
                ls_index,
                address,
                method,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let last_block = client.get_masterchain_info().await?.last;
                let method_id = crate::utils::method_name_to_id(method);
                let result = client
                    .run_get_method(
                        0,
                        last_block.clone(),
                        Address::from_str(address)?,
                        method_id,
                        TvmStack::empty(),
                    )
                    .await?;
                let _ = last_block;
                self.print_structured(&run_get_method_view(
                    result,
                    Some(method.clone()),
                    method_id,
                )?)
            }
            LiteClientCommand::RawGetBlock { ls_index, block } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = client
                    .raw_get_block_data(parse_block_id_ext(block)?)
                    .await?;
                self.print_structured(&decoded_block_data_value(&decoded))
            }
            LiteClientCommand::RawGetBlockHeader {
                ls_index,
                block,
                with_state_update,
                with_value_flow,
                with_extra,
                with_shard_hashes,
                with_prev_blk_signatures,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = client
                    .raw_get_block_header(
                        parse_block_id_ext(block)?,
                        *with_state_update,
                        *with_value_flow,
                        *with_extra,
                        *with_shard_hashes,
                        *with_prev_blk_signatures,
                    )
                    .await?;
                self.print_structured(&decoded_block_header_value(&decoded))
            }
            LiteClientCommand::GetAccountStateTyped {
                ls_index,
                address,
                block,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let address_value = Address::from_str(address)?;
                let latest = client.get_masterchain_info().await?.last;
                let block = latest_or_explicit_block(block.as_ref(), latest)?;
                let raw = client
                    .get_account_state(block, address_value.to_account_id())
                    .await?;
                self.print_account(&best_effort_account_state_view(address, raw))
            }
            LiteClientCommand::RawGetAccountState {
                ls_index,
                address,
                block,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let block = block
                    .as_ref()
                    .map(|value| parse_block_id_ext(value))
                    .transpose()?;
                let (account, shard_account) = client
                    .raw_get_account_state(Address::from_str(address)?, block)
                    .await?;
                self.print_structured(&json!({
                    "account": account.as_ref().map(account_value),
                    "shard_account": shard_account.as_ref().map(shard_account_value),
                }))
            }
            LiteClientCommand::GetAccountStateSimple { ls_index, address } => {
                let mut client = self.create_client(*ls_index).await?;
                let account = client
                    .get_account_state_simple(Address::from_str(address)?)
                    .await?;
                self.print_structured(&simple_account_value(&account))
            }
            LiteClientCommand::RawGetShardInfo {
                ls_index,
                block,
                workchain,
                shard,
                exact,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = client
                    .raw_get_shard_info(
                        parse_block_id_ext(block)?,
                        *workchain,
                        parse_u64_decimal_or_hex(shard)?,
                        *exact,
                    )
                    .await?;
                self.print_structured(&decoded_shard_info_value(&decoded))
            }
            LiteClientCommand::RawGetAllShardsInfo { ls_index, block } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = client
                    .raw_get_all_shards_info(parse_block_id_ext(block)?)
                    .await?;
                self.print_structured(&decoded_all_shards_info_value(&decoded))
            }
            LiteClientCommand::GetAllShardsInfoTyped { ls_index, block } => {
                let mut client = self.create_client(*ls_index).await?;
                let shards = client
                    .get_all_shards_info_typed(parse_block_id_ext(block)?)
                    .await?;
                self.print_structured(&json!({
                    "shards": shards.iter().map(block_id_ext_view).collect::<Vec<_>>()
                }))
            }
            LiteClientCommand::GetOneTransactionTyped {
                ls_index,
                block,
                account,
                lt,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let transaction = client
                    .get_one_transaction_typed(
                        parse_block_id_ext(block)?,
                        parse_account_id(account)?,
                        *lt,
                    )
                    .await?;
                self.print_structured(
                    &json!({ "transaction": transaction.as_ref().map(transaction_value) }),
                )
            }
            LiteClientCommand::RawGetTransactions {
                ls_index,
                account,
                lt,
                hash,
                count,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let (transactions, ids) = client
                    .raw_get_transactions(
                        *count,
                        parse_account_id(account)?,
                        *lt,
                        parse_int256(hash)?,
                    )
                    .await?;
                self.print_structured(&json!({
                    "ids": ids.iter().map(block_id_ext_view).collect::<Vec<_>>(),
                    "transactions": transactions_value(&transactions),
                }))
            }
            LiteClientCommand::RawGetBlockTransactionsExt {
                ls_index,
                block,
                count,
                after_account,
                after_lt,
                reverse_order,
                want_proof,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = client
                    .list_block_transactions_ext_decoded(
                        parse_block_id_ext(block)?,
                        *count,
                        parse_after_transaction(after_account, *after_lt)?,
                        *reverse_order,
                        *want_proof,
                    )
                    .await?;
                self.print_structured(&json!({
                    "id": block_id_ext_view(&decoded.raw.id),
                    "transactions": transactions_value(&decoded.transactions),
                    "proof": decoded.proof.as_ref().map(decoded_boc_view),
                }))
            }
            LiteClientCommand::RunGetMethodTyped {
                ls_index,
                address,
                block,
                method,
                method_id,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let last = client.get_masterchain_info().await?.last;
                let block = latest_or_explicit_block(block.as_ref(), last)?;
                let method_id = method_id
                    .or_else(|| method.as_deref().map(crate::utils::method_name_to_id))
                    .context("run-get-method-typed requires --method or --method-id")?;
                let stack = client
                    .run_get_method_typed(
                        0,
                        block.clone(),
                        Address::from_str(address)?,
                        method_id,
                        TvmStack::empty(),
                    )
                    .await?;
                self.print_structured(&json!({
                    "block": block_id_ext_view(&block),
                    "method": method,
                    "method_id": method_id,
                    "stack": TvmStackView {
                        entries: stack.iter().map(stack_entry_view).collect::<Result<Vec<_>>>()?,
                    },
                }))
            }
            LiteClientCommand::GetConfigAllTyped {
                ls_index,
                block,
                flags,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded =
                    get_config_all_client(&mut client, parse_block_id_ext(block)?, flags).await?;
                self.print_structured(&decoded_config_info_value(&decoded))
            }
            LiteClientCommand::GetConfigParamsTyped {
                ls_index,
                block,
                params,
                flags,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = get_config_params_client(
                    &mut client,
                    parse_block_id_ext(block)?,
                    parse_params(params)?,
                    flags,
                )
                .await?;
                self.print_structured(&decoded_config_info_value(&decoded))
            }
            LiteClientCommand::GetLibrariesTyped {
                ls_index,
                libraries,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let libraries = client
                    .get_libraries_typed(parse_libraries(libraries)?)
                    .await?;
                self.print_structured(&libraries_value(&libraries))
            }
            LiteClientCommand::GetLibrariesWithProofTyped {
                ls_index,
                block,
                libraries,
                mode,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let decoded = client
                    .get_libraries_with_proof_typed(
                        parse_block_id_ext(block)?,
                        *mode,
                        parse_libraries(libraries)?,
                    )
                    .await?;
                self.print_structured(&decoded_libraries_with_proof_value(&decoded))
            }
        }
    }

    async fn execute_contract(&self, command: &ContractCommand) -> Result<()> {
        match command {
            ContractCommand::State { ls_index, address } => {
                let mut client = self.create_client(*ls_index).await?;
                let address = Address::from_str(address)?;
                let mut contract = Contract::new(&mut client, address);
                let state = contract.get_state_latest().await?;
                self.print_structured(&account_state_view(state))
            }
            ContractCommand::RunGetMethod {
                ls_index,
                address,
                method,
                method_id,
            } => {
                let mut client = self.create_client(*ls_index).await?;
                let address = Address::from_str(address)?;
                let method_id = method_id
                    .or_else(|| method.as_deref().map(crate::utils::method_name_to_id))
                    .context("run-get-method requires --method or --method-id")?;
                let mut contract = Contract::new(&mut client, address);
                let result = contract
                    .run_get_method_latest(method_id, TvmStack::empty())
                    .await?;
                self.print_structured(&run_get_method_view(result, method.clone(), method_id)?)
            }
        }
    }

    async fn execute_balancer(&self, command: &BalancerCommand) -> Result<()> {
        match command {
            BalancerCommand::MasterchainInfo { num_servers } => {
                let mut balancer = self.create_balancer(*num_servers).await?;
                let info = balancer.get_masterchain_info().await?;
                balancer.close_all().await?;
                self.print_structured(&masterchain_info_view(info))
            }
            BalancerCommand::Status { num_servers } => {
                let mut balancer = self.create_balancer(*num_servers).await?;
                let status = BalancerStatusView {
                    total_peers: balancer.peers_num(),
                    alive_peers: balancer.alive_peers_num().await,
                    archival_peers: balancer.archival_peers_num().await,
                };
                balancer.close_all().await?;
                self.print_structured(&status)
            }
            BalancerCommand::RawGetBlock(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = balancer
                    .raw_get_block_data(parse_block_id_ext(&args.block)?)
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_block_data_value(&decoded))
            }
            BalancerCommand::RawGetBlockHeader(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = balancer
                    .raw_get_block_header(
                        parse_block_id_ext(&args.block)?,
                        args.with_state_update,
                        args.with_value_flow,
                        args.with_extra,
                        args.with_shard_hashes,
                        args.with_prev_blk_signatures,
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_block_header_value(&decoded))
            }
            BalancerCommand::GetAccountStateTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let address = Address::from_str(&args.address)?;
                let latest = balancer.get_masterchain_info().await?.last;
                let block = latest_or_explicit_block(args.block.as_ref(), latest)?;
                let raw = balancer
                    .get_account_state(block, address.to_account_id())
                    .await?;
                balancer.close_all().await?;
                self.print_account(&best_effort_account_state_view(&args.address, raw))
            }
            BalancerCommand::RawGetAccountState(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let block = args
                    .block
                    .as_ref()
                    .map(|value| parse_block_id_ext(value))
                    .transpose()?;
                let (account, shard_account) = balancer
                    .raw_get_account_state(Address::from_str(&args.address)?, block)
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&json!({
                    "account": account.as_ref().map(account_value),
                    "shard_account": shard_account.as_ref().map(shard_account_value),
                }))
            }
            BalancerCommand::GetAccountStateSimple(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let account = balancer
                    .get_account_state_simple(Address::from_str(&args.address)?)
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&simple_account_value(&account))
            }
            BalancerCommand::RawGetShardInfo(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = balancer
                    .raw_get_shard_info(
                        parse_block_id_ext(&args.block)?,
                        args.workchain,
                        parse_u64_decimal_or_hex(&args.shard)?,
                        args.exact,
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_shard_info_value(&decoded))
            }
            BalancerCommand::RawGetAllShardsInfo(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = balancer
                    .raw_get_all_shards_info(parse_block_id_ext(&args.block)?)
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_all_shards_info_value(&decoded))
            }
            BalancerCommand::GetAllShardsInfoTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let shards = balancer
                    .get_all_shards_info_typed(parse_block_id_ext(&args.block)?)
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&json!({
                    "shards": shards.iter().map(block_id_ext_view).collect::<Vec<_>>()
                }))
            }
            BalancerCommand::GetOneTransactionTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let transaction = balancer
                    .get_one_transaction_typed(
                        parse_block_id_ext(&args.block)?,
                        parse_account_id(&args.account)?,
                        args.lt,
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(
                    &json!({ "transaction": transaction.as_ref().map(transaction_value) }),
                )
            }
            BalancerCommand::RawGetTransactions(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let (transactions, ids) = balancer
                    .raw_get_transactions(
                        args.count,
                        parse_account_id(&args.account)?,
                        args.lt,
                        parse_int256(&args.hash)?,
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&json!({
                    "ids": ids.iter().map(block_id_ext_view).collect::<Vec<_>>(),
                    "transactions": transactions_value(&transactions),
                }))
            }
            BalancerCommand::RawGetBlockTransactionsExt(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = balancer
                    .list_block_transactions_ext_decoded(
                        parse_block_id_ext(&args.block)?,
                        args.count,
                        parse_after_transaction(&args.after_account, args.after_lt)?,
                        args.reverse_order,
                        args.want_proof,
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&json!({
                    "id": block_id_ext_view(&decoded.raw.id),
                    "transactions": transactions_value(&decoded.transactions),
                    "proof": decoded.proof.as_ref().map(decoded_boc_view),
                }))
            }
            BalancerCommand::RunGetMethodTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let last = balancer.get_masterchain_info().await?.last;
                let block = latest_or_explicit_block(args.block.as_ref(), last)?;
                let method_id = args
                    .method_id
                    .or_else(|| args.method.as_deref().map(crate::utils::method_name_to_id))
                    .context("run-get-method-typed requires --method or --method-id")?;
                let stack = balancer
                    .run_get_method_typed(
                        0,
                        block.clone(),
                        Address::from_str(&args.address)?,
                        method_id,
                        TvmStack::empty(),
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&json!({
                    "block": block_id_ext_view(&block),
                    "method": args.method,
                    "method_id": method_id,
                    "stack": TvmStackView {
                        entries: stack.iter().map(stack_entry_view).collect::<Result<Vec<_>>>()?,
                    },
                }))
            }
            BalancerCommand::GetConfigAllTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = get_config_all_balancer(
                    &mut balancer,
                    parse_block_id_ext(&args.block)?,
                    &args.flags,
                )
                .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_config_info_value(&decoded))
            }
            BalancerCommand::GetConfigParamsTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = get_config_params_balancer(
                    &mut balancer,
                    parse_block_id_ext(&args.block)?,
                    parse_params(&args.params)?,
                    &args.flags,
                )
                .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_config_info_value(&decoded))
            }
            BalancerCommand::GetLibrariesTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let libraries = balancer
                    .get_libraries_typed(parse_libraries(&args.libraries)?)
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&libraries_value(&libraries))
            }
            BalancerCommand::GetLibrariesWithProofTyped(args) => {
                let mut balancer = self.create_balancer(args.num_servers).await?;
                let decoded = balancer
                    .get_libraries_with_proof_typed(
                        parse_block_id_ext(&args.block)?,
                        args.mode,
                        parse_libraries(&args.libraries)?,
                    )
                    .await?;
                balancer.close_all().await?;
                self.print_structured(&decoded_libraries_with_proof_value(&decoded))
            }
        }
    }

    fn print_status(&self, value: &StatusView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("network: {}", value.network.name);
                println!("backend: {}", value.backend.mode);
                if let Some(ls_index) = value.backend.ls_index {
                    println!("ls_index: {ls_index}");
                }
                if let Some(num_servers) = value.backend.num_servers {
                    println!("num_servers: {num_servers}");
                }
                print_block_human("latest", &value.latest);
                if let Some(peers) = &value.peers {
                    println!("peers_total: {}", peers.total_peers);
                    println!("peers_alive: {}", peers.alive_peers);
                    println!("peers_archival: {}", peers.archival_peers);
                }
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_account(&self, value: &BestEffortAccountStateView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("address: {}", value.address);
                println!("state: {}", value.state);
                if let Some(balance) = &value.balance {
                    println!("balance: {balance}");
                }
                if let Some(lt) = value.last_transaction_lt {
                    println!("last_transaction_lt: {lt}");
                }
                if let Some(hash) = &value.last_transaction_hash {
                    println!("last_transaction_hash: {hash}");
                }
                print_block_human("block", &value.block);
                print_block_human("shard_block", &value.shard_block);
                println!("state_len: {}", value.state_len);
                println!("shard_proof_len: {}", value.shard_proof_len);
                println!("proof_len: {}", value.proof_len);
                if let Some(hash) = &value.state_root_hash {
                    println!("state_root_hash: {hash}");
                }
                if let Some(count) = value.shard_proof_root_count {
                    println!("shard_proof_root_count: {count}");
                }
                for hash in &value.shard_proof_root_hashes {
                    println!("shard_proof_root_hash: {hash}");
                }
                if let Some(count) = value.proof_root_count {
                    println!("proof_root_count: {count}");
                }
                for hash in &value.proof_root_hashes {
                    println!("proof_root_hash: {hash}");
                }
                for error in &value.decode_errors {
                    println!("decode_error: {error}");
                }
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_call(&self, value: &HighLevelCallView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("address: {}", value.address);
                if let Some(method) = &value.method {
                    println!("method: {method}");
                }
                println!("method_id: {}", value.method_id);
                println!("exit_code: {}", value.exit_code);
                print_block_human("block", &value.block);
                print_block_human("shard_block", &value.shard_block);
                if let Some(stack) = &value.stack {
                    println!("stack_entries: {}", stack.entries.len());
                    for (index, entry) in stack.entries.iter().enumerate() {
                        println!("stack[{index}]: {}", stack_entry_human(entry));
                    }
                }
                for error in &value.decode_errors {
                    println!("decode_error: {error}");
                }
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_transactions(&self, value: &HighLevelTransactionsView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("address: {}", value.address);
                println!("requested_count: {}", value.count);
                println!("transactions: {}", value.transactions.len());
                for error in &value.decode_errors {
                    println!("decode_error: {error}");
                }
                if value.transactions.is_empty() {
                    return Ok(());
                }
                println!("lt\tutc_time\tstatus\tout_msgs");
                for tx in &value.transactions {
                    println!(
                        "{}\t{}\t{}->{}\t{}",
                        tx.get("lt").and_then(Value::as_u64).unwrap_or_default(),
                        tx.get("now").and_then(Value::as_u64).unwrap_or_default(),
                        tx.get("orig_status")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                        tx.get("end_status")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                        tx.get("outmsg_cnt")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                    );
                }
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_structured<T: Serialize>(&self, value: &T) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("{}", serde_json::to_string_pretty(value)?);
                Ok(())
            }
            OutputFormat::Json => {
                println!("{}", serde_json::to_string(value)?);
                Ok(())
            }
            OutputFormat::PrettyJson => {
                println!("{}", serde_json::to_string_pretty(value)?);
                Ok(())
            }
            OutputFormat::Raw | OutputFormat::Hex | OutputFormat::Base64 => {
                anyhow::bail!("selected output format is only valid for byte output")
            }
        }
    }

    fn print_bytes(&self, bytes: &[u8]) -> Result<()> {
        match self.output {
            OutputFormat::Raw => {
                io::stdout().write_all(bytes)?;
                Ok(())
            }
            OutputFormat::Base64 => {
                println!(
                    "{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                );
                Ok(())
            }
            OutputFormat::Human | OutputFormat::Hex => {
                println!("{}", hex::encode(bytes));
                Ok(())
            }
            OutputFormat::Json => self.print_raw_json(bytes, false),
            OutputFormat::PrettyJson => self.print_raw_json(bytes, true),
        }
    }

    fn print_raw_json(&self, bytes: &[u8], pretty: bool) -> Result<()> {
        let value = raw_bytes_view(bytes);
        if pretty {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            println!("{}", serde_json::to_string(&value)?);
        }
        Ok(())
    }
}

fn decode_known_tlb(
    root: std::sync::Arc<Cell>,
    known: Option<KnownTlbType>,
    verify_proof: bool,
) -> Result<(Option<String>, Option<Value>, Option<bool>)> {
    let Some(known) = known else {
        return Ok((None, None, None));
    };

    let name = format!("{known:?}");
    let (value, verified) = match known {
        KnownTlbType::Message => {
            let _ = crate::tlb::Message::from_cell(root)?;
            (json!({ "type": "message" }), None)
        }
        KnownTlbType::MessageRelaxed => {
            let _ = crate::tlb::MessageRelaxed::from_cell(root)?;
            (json!({ "type": "message_relaxed" }), None)
        }
        KnownTlbType::Transaction => (
            transaction_value(&crate::tlb::Transaction::from_cell(root)?),
            None,
        ),
        KnownTlbType::Account => (account_value(&crate::tlb::Account::from_cell(root)?), None),
        KnownTlbType::Block => (block_value(&crate::tlb::Block::from_cell(root)?), None),
        KnownTlbType::Config => (
            config_params_value(&crate::tlb::ConfigParams::from_cell(root)?),
            None,
        ),
        KnownTlbType::ShardState => (
            shard_state_value(&crate::tlb::ShardState::from_cell(root)?),
            None,
        ),
        KnownTlbType::Proof => {
            let proof = crate::tlb::MerkleProof::from_exotic_cell(root)?;
            let verified = verify_proof.then(|| proof.verify_virtual_hash());
            (
                json!({
                    "type": "merkle_proof",
                    "cell": cell_value(&proof.cell),
                    "virtual_hash": hex::encode(proof.virtual_hash),
                    "depth": proof.depth,
                    "virtual_root": cell_value(&proof.virtual_root),
                }),
                verified,
            )
        }
        KnownTlbType::MerkleUpdate => {
            let update = crate::tlb::MerkleUpdate::from_exotic_cell(root)?;
            let verified = verify_proof.then(|| update.verify_virtual_hashes());
            (
                json!({
                    "type": "merkle_update",
                    "cell": cell_value(&update.cell),
                    "old_hash": hex::encode(update.old_hash),
                    "new_hash": hex::encode(update.new_hash),
                    "old_depth": update.old_depth,
                    "new_depth": update.new_depth,
                    "old": cell_value(&update.old),
                    "new": cell_value(&update.new),
                }),
                verified,
            )
        }
    };
    Ok((Some(name), Some(value), verified))
}

fn read_raw_input(
    hex_input: &Option<String>,
    base64_input: &Option<String>,
    file: &Option<String>,
    stdin: bool,
) -> Result<Vec<u8>> {
    match (hex_input, base64_input, file, stdin) {
        (Some(value), None, None, false) => {
            hex::decode(value.trim()).context("failed to decode hex input")
        }
        (None, Some(value), None, false) => base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .context("failed to decode base64 input"),
        (None, None, Some(path), false) => {
            fs::read(path).with_context(|| format!("failed to read input file {path}"))
        }
        (None, None, None, true) => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        (None, None, None, false) => {
            anyhow::bail!("requires one of --hex, --base64, --file, or --stdin")
        }
        _ => anyhow::bail!("accepts only one input source"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_debug_asserts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_liteclient_json_command() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--output",
            "json",
            "--rps",
            "5",
            "liteclient",
            "masterchain-info",
            "--ls-index",
            "2",
        ])
        .unwrap();

        assert_eq!(cli.output, OutputFormat::Json);
        assert_eq!(cli.rps.map(NonZeroU32::get), Some(5));
        match cli.command {
            Commands::Liteclient {
                command: LiteClientCommand::MasterchainInfo { ls_index },
            } => assert_eq!(ls_index, 2),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_balancer_global_rps() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--global-rps",
            "10",
            "balancer",
            "masterchain-info",
            "--num-servers",
            "2",
        ])
        .unwrap();

        assert_eq!(cli.global_rps.map(NonZeroU32::get), Some(10));
        match cli.command {
            Commands::Balancer {
                command: BalancerCommand::MasterchainInfo { num_servers },
            } => assert_eq!(num_servers, 2),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_high_level_commands() {
        let block = "0:0x8000000000000000:1:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222";
        let address = "0:3333333333333333333333333333333333333333333333333333333333333333";

        let status = Cli::try_parse_from([
            "tonutils",
            "--num-servers",
            "2",
            "--single",
            "--ls-index",
            "1",
            "status",
        ])
        .unwrap();
        assert!(status.single);
        assert_eq!(status.num_servers, 2);
        assert_eq!(status.ls_index, 1);

        for args in [
            vec!["account", address, "--block", block],
            vec![
                "call", address, "seqno", "--arg", "int:1", "--arg", "null", "--block", block,
            ],
            vec!["call", address, "85143"],
            vec!["transactions", address, "--count", "5"],
            vec!["block", "latest"],
            vec!["block", "get", block],
            vec!["config", "get", "--params", "0,17", "--block", block],
            vec!["config", "get"],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn parses_stack_args() {
        let stack = parse_stack_args(&["int:-5".to_owned(), "null".to_owned()]).unwrap();
        assert_eq!(stack.entries().len(), 2);
        assert!(matches!(stack.entries()[0], TvmStackEntry::Int(_)));
        assert!(matches!(stack.entries()[1], TvmStackEntry::Null));

        let cell = crate::tvm::CellBuilder::new().build().unwrap();
        let boc = hex::encode(crate::tvm::serialize_boc(&cell, false).unwrap());
        assert!(matches!(
            parse_stack_arg(&format!("cell:{boc}")).unwrap(),
            TvmStackEntry::Cell(_)
        ));
        assert!(matches!(
            parse_stack_arg(&format!("slice:{boc}")).unwrap(),
            TvmStackEntry::Slice(_)
        ));

        assert!(parse_stack_arg("bad").is_err());
        assert!(parse_stack_arg("uint:1").is_err());
        assert!(parse_stack_arg("int:not-a-number").is_err());
        assert!(parse_stack_arg("cell:00").is_err());
    }

    #[test]
    fn rejects_zero_rps() {
        assert!(Cli::try_parse_from(["tonutils", "--rps", "0", "liteclient", "time"]).is_err());
    }

    #[test]
    fn rejects_zero_global_rps() {
        assert!(
            Cli::try_parse_from(["tonutils", "--global-rps", "0", "balancer", "status"]).is_err()
        );
    }

    #[test]
    fn raw_input_decodes_hex() {
        let bytes = read_raw_input(&Some("0a0b0c".to_owned()), &None, &None, false).unwrap();
        assert_eq!(bytes, vec![10, 11, 12]);
    }

    #[test]
    fn parses_contract_state_command() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--output",
            "json",
            "contract",
            "state",
            "--ls-index",
            "1",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command: ContractCommand::State { ls_index, address },
            } => {
                assert_eq!(ls_index, 1);
                assert!(address.starts_with("0:11"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_contract_run_get_method_by_name() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "contract",
            "run-get-method",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            "--method",
            "seqno",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command:
                    ContractCommand::RunGetMethod {
                        method, method_id, ..
                    },
            } => {
                assert_eq!(method.as_deref(), Some("seqno"));
                assert_eq!(method_id, None);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_contract_run_get_method_by_id() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "contract",
            "run-get-method",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            "--method-id",
            "85143",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command:
                    ContractCommand::RunGetMethod {
                        method, method_id, ..
                    },
            } => {
                assert_eq!(method, None);
                assert_eq!(method_id, Some(85143));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_tvm_boc_decode_with_tlb_type() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--output",
            "json",
            "tvm",
            "boc",
            "decode",
            "--hex",
            "b5ee9c72010101010002000000",
            "--tlb",
            "account",
        ])
        .unwrap();

        match cli.command {
            Commands::Tvm {
                command:
                    TvmCommand::Boc {
                        command: BocCommand::Decode { tlb, .. },
                    },
            } => assert_eq!(tlb, Some(KnownTlbType::Account)),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_tvm_schema_check() {
        let cli = Cli::try_parse_from(["tonutils", "tvm", "schema", "check"]).unwrap();

        match cli.command {
            Commands::Tvm {
                command:
                    TvmCommand::Schema {
                        command: SchemaCommand::Check,
                    },
            } => {}
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_new_liteclient_commands() {
        let block = "0:0x8000000000000000:1:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222";
        let account = "0:3333333333333333333333333333333333333333333333333333333333333333";
        let address = account;
        let hash = "4444444444444444444444444444444444444444444444444444444444444444";

        for args in [
            vec!["liteclient", "raw-get-block", "--block", block],
            vec![
                "liteclient",
                "raw-get-block-header",
                "--block",
                block,
                "--with-state-update",
                "--with-value-flow",
                "--with-extra",
                "--with-shard-hashes",
                "--with-prev-blk-signatures",
            ],
            vec![
                "liteclient",
                "get-account-state-typed",
                "--address",
                address,
                "--block",
                block,
            ],
            vec!["liteclient", "raw-get-account-state", "--address", address],
            vec![
                "liteclient",
                "get-account-state-simple",
                "--address",
                address,
            ],
            vec![
                "liteclient",
                "raw-get-shard-info",
                "--block",
                block,
                "--workchain",
                "0",
                "--shard",
                "0x8000000000000000",
                "--exact",
            ],
            vec!["liteclient", "raw-get-all-shards-info", "--block", block],
            vec!["liteclient", "get-all-shards-info-typed", "--block", block],
            vec![
                "liteclient",
                "get-one-transaction-typed",
                "--block",
                block,
                "--account",
                account,
                "--lt",
                "7",
            ],
            vec![
                "liteclient",
                "raw-get-transactions",
                "--account",
                account,
                "--lt",
                "7",
                "--hash",
                hash,
                "--count",
                "3",
            ],
            vec![
                "liteclient",
                "raw-get-block-transactions-ext",
                "--block",
                block,
                "--count",
                "3",
                "--after-account",
                hash,
                "--after-lt",
                "7",
                "--reverse-order",
                "--want-proof",
            ],
            vec![
                "liteclient",
                "run-get-method-typed",
                "--address",
                address,
                "--method",
                "seqno",
            ],
            vec![
                "liteclient",
                "get-config-all-typed",
                "--block",
                block,
                "--with-state-root",
            ],
            vec![
                "liteclient",
                "get-config-params-typed",
                "--block",
                block,
                "--params",
                "0,1,-1",
                "--with-libraries",
            ],
            vec!["liteclient", "get-libraries-typed", "--libraries", hash],
            vec![
                "liteclient",
                "get-libraries-with-proof-typed",
                "--block",
                block,
                "--libraries",
                hash,
                "--mode",
                "1",
            ],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn parses_new_balancer_commands() {
        let block = "0:9223372036854775808:1:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222";
        let account = "0:3333333333333333333333333333333333333333333333333333333333333333";
        let hash = "4444444444444444444444444444444444444444444444444444444444444444";

        for args in [
            vec!["balancer", "raw-get-block", "--block", block],
            vec!["balancer", "raw-get-block-header", "--block", block],
            vec!["balancer", "get-account-state-typed", "--address", account],
            vec!["balancer", "raw-get-account-state", "--address", account],
            vec!["balancer", "get-account-state-simple", "--address", account],
            vec![
                "balancer",
                "raw-get-shard-info",
                "--block",
                block,
                "--workchain",
                "0",
                "--shard",
                "1",
            ],
            vec!["balancer", "raw-get-all-shards-info", "--block", block],
            vec!["balancer", "get-all-shards-info-typed", "--block", block],
            vec![
                "balancer",
                "get-one-transaction-typed",
                "--block",
                block,
                "--account",
                account,
                "--lt",
                "7",
            ],
            vec![
                "balancer",
                "raw-get-transactions",
                "--account",
                account,
                "--lt",
                "7",
                "--hash",
                hash,
                "--count",
                "3",
            ],
            vec![
                "balancer",
                "raw-get-block-transactions-ext",
                "--block",
                block,
                "--count",
                "3",
            ],
            vec![
                "balancer",
                "run-get-method-typed",
                "--address",
                account,
                "--method-id",
                "85143",
            ],
            vec!["balancer", "get-config-all-typed", "--block", block],
            vec![
                "balancer",
                "get-config-params-typed",
                "--block",
                block,
                "--params",
                "0,1",
            ],
            vec!["balancer", "get-libraries-typed", "--libraries", hash],
            vec![
                "balancer",
                "get-libraries-with-proof-typed",
                "--block",
                block,
                "--libraries",
                hash,
            ],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_typed_cli_inputs() {
        assert!(parse_block_id_ext("0:1:2:abcd").is_err());
        assert!(parse_block_id_ext("0:1:2:abcd:00").is_err());
        assert!(parse_params("1,,2").is_err());
        assert!(parse_libraries("abcd").is_err());
        assert!(parse_after_transaction(&Some("11".to_owned()), None).is_err());

        assert!(
            Cli::try_parse_from([
                "tonutils",
                "liteclient",
                "run-get-method-typed",
                "--address",
                "0:1111111111111111111111111111111111111111111111111111111111111111",
                "--method",
                "seqno",
                "--method-id",
                "85143",
            ])
            .is_err()
        );
    }

    #[test]
    fn account_state_json_view_contains_lengths_and_raw_state() {
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let view = account_state_view(crate::tl::response::AccountState {
            id: block.clone(),
            shardblk: block,
            shard_proof: vec![1, 2],
            proof: vec![3],
            state: vec![4, 5, 6],
        });

        assert_eq!(view.shard_proof_len, 2);
        assert_eq!(view.proof_len, 1);
        assert_eq!(view.state.hex, "040506");
    }

    #[test]
    fn best_effort_account_state_accepts_multi_root_proofs() {
        use crate::tlb::TlbSerialize;

        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let account = crate::tlb::Account::None;
        let state = crate::tvm::serialize_boc(&account.to_cell().unwrap(), false).unwrap();
        let proof = hex::decode("b5ee9c72010102020005000100000002aa").unwrap();
        let view = best_effort_account_state_view(
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            crate::tl::response::AccountState {
                id: block.clone(),
                shardblk: block,
                shard_proof: proof.clone(),
                proof,
                state,
            },
        );

        assert_eq!(view.state, "none");
        assert_eq!(view.shard_proof_root_count, Some(2));
        assert_eq!(view.proof_root_count, Some(2));
        assert_eq!(view.shard_proof_root_hashes.len(), 2);
        assert!(view.decode_errors.is_empty());
    }

    #[test]
    fn run_get_method_json_view_decodes_supported_stack() {
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let result = crate::tl::response::RunMethodResult {
            mode: (),
            id: block.clone(),
            shardblk: block,
            shard_proof: Some(vec![1, 2]),
            proof: None,
            state_proof: Some(vec![3, 4, 5]),
            init_c7: None,
            lib_extras: None,
            exit_code: 0,
            result: Some(TvmStack::new(vec![TvmStackEntry::int(5)]).to_boc().unwrap()),
        };

        let view = run_get_method_view(result, Some("seqno".to_owned()), 85143).unwrap();

        assert_eq!(view.method.as_deref(), Some("seqno"));
        assert_eq!(view.shard_proof_len, 2);
        assert_eq!(view.state_proof_len, 3);
        assert!(view.decoded_stack.is_some());
        assert!(view.result_decode_error.is_none());
    }
}
