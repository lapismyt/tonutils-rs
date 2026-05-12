use crate::contracts::{Contract, DecodedRunMethodResult, RunMethodResultExt};
use crate::liteclient::{balancer::LiteBalancer, client::LiteClient, rate_limit::RequestRateLimit};
use crate::network_config::ConfigGlobal;
use crate::tl::{AccountId, BlockIdExt, Int256, common::TransactionId3};
use crate::tlb::TlbDeserialize;
use crate::tvm::{Builder, Cell, TvmStack, TvmStackEntry, address::Address};
use crate::wallet::{
    MAINNET_GLOBAL_ID, TESTNET_GLOBAL_ID, TonMnemonic, WALLET_V4R2_DEFAULT_ID, WalletMessage,
    WalletV4R2, WalletV5R1, WalletV5R1WalletId, wallet_v4r2_code, wallet_v5r1_code,
};
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
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletVersionArg {
    V4R2,
    V5R1,
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
    /// Wallet generation, address derivation, transfer preparation, and send.
    Wallet {
        #[command(subcommand)]
        command: WalletCommand,
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

#[derive(Subcommand, Debug)]
pub enum WalletCommand {
    /// Generate a 24-word TON mnemonic and print derived wallet addresses.
    Generate {
        /// Workchain used for derived addresses.
        #[arg(long, default_value = "0")]
        workchain: i8,
        /// Read optional mnemonic password from this environment variable.
        #[arg(long)]
        mnemonic_password_env: Option<String>,
    },
    /// Derive a wallet address from a mnemonic read from file, env, or stdin.
    Address(WalletAddressArgs),
    /// Fetch wallet seqno via get-method at latest masterchain block.
    Seqno {
        /// Wallet address in raw or friendly form.
        address: String,
    },
    /// Build a signed external transfer message BoC without sending it.
    PrepareTransfer(WalletTransferArgs),
    /// Build and send a signed external transfer message BoC.
    Send(WalletTransferArgs),
}

#[derive(Parser, Debug)]
pub struct WalletAddressArgs {
    /// Wallet version to derive.
    #[arg(long, default_value = "v5r1")]
    version: WalletVersionArg,
    /// Workchain used for derived address.
    #[arg(long, default_value = "0")]
    workchain: i8,
    /// Override wallet id.
    #[arg(long)]
    wallet_id: Option<u32>,
    /// Read mnemonic phrase from a file, or from stdin when set to '-'.
    #[arg(long, conflicts_with = "mnemonic_env")]
    mnemonic_file: Option<String>,
    /// Read mnemonic phrase from an environment variable.
    #[arg(long, conflicts_with = "mnemonic_file")]
    mnemonic_env: Option<String>,
    /// Read optional mnemonic password from this environment variable.
    #[arg(long)]
    mnemonic_password_env: Option<String>,
}

#[derive(Parser, Debug)]
pub struct WalletTransferArgs {
    /// Wallet version to use.
    #[arg(long, default_value = "v5r1")]
    version: WalletVersionArg,
    /// Destination address in raw or friendly form.
    #[arg(long)]
    to: String,
    /// Amount in nanotons.
    #[arg(long)]
    amount: u64,
    /// Optional text comment stored as a standard comment body.
    #[arg(long)]
    comment: Option<String>,
    /// Send mode for the internal transfer.
    #[arg(long, default_value = "3")]
    mode: u8,
    /// Message timeout in seconds. Used to compute valid_until from local time.
    #[arg(long, default_value = "60")]
    timeout: u32,
    /// Override wallet seqno. Required for offline prepare-transfer.
    #[arg(long)]
    seqno: Option<u32>,
    /// Override wallet id.
    #[arg(long)]
    wallet_id: Option<u32>,
    /// Wallet workchain.
    #[arg(long, default_value = "0")]
    workchain: i8,
    /// Include StateInit in the external message.
    #[arg(long)]
    deploy: bool,
    /// Read mnemonic phrase from a file, or from stdin when set to '-'.
    #[arg(long, conflicts_with = "mnemonic_env")]
    mnemonic_file: Option<String>,
    /// Read mnemonic phrase from an environment variable.
    #[arg(long, conflicts_with = "mnemonic_file")]
    mnemonic_env: Option<String>,
    /// Read optional mnemonic password from this environment variable.
    #[arg(long)]
    mnemonic_password_env: Option<String>,
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

