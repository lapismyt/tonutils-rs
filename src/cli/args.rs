use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::num::NonZeroU32;

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
    /// Exclude liteservers from balancer paths by config index or public key id.
    #[arg(long = "exclude-ls", global = true, value_delimiter = ',')]
    pub exclude_ls: Vec<String>,
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
    /// Stack argument: null, int:<decimal>, cell:<boc-hex>, slice:<boc-hex>, unsupported:<hex>, tuple:<json-array>, or list:<json-array>.
    #[arg(long = "arg", conflicts_with = "stack_json")]
    pub args: Vec<String>,
    /// Inline JSON stack array for scriptable nested input.
    #[arg(long, conflicts_with = "args")]
    pub stack_json: Option<String>,
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
    pub(super) with_state_root: bool,
    #[arg(long)]
    pub(super) with_libraries: bool,
    #[arg(long)]
    pub(super) with_state_extra_root: bool,
    #[arg(long)]
    pub(super) with_shard_hashes: bool,
    #[arg(long)]
    pub(super) with_validator_set: bool,
    #[arg(long)]
    pub(super) with_special_smc: bool,
    #[arg(long)]
    pub(super) with_accounts_root: bool,
    #[arg(long)]
    pub(super) with_prev_blocks: bool,
    #[arg(long)]
    pub(super) with_workchain_info: bool,
    #[arg(long)]
    pub(super) with_capabilities: bool,
    #[arg(long)]
    pub(super) extract_from_key_block: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerBlockArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
}

#[derive(Parser, Debug)]
pub struct BalancerHeaderArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[arg(long)]
    pub(super) with_state_update: bool,
    #[arg(long)]
    pub(super) with_value_flow: bool,
    #[arg(long)]
    pub(super) with_extra: bool,
    #[arg(long)]
    pub(super) with_shard_hashes: bool,
    #[arg(long)]
    pub(super) with_prev_blk_signatures: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerAddressBlockArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) address: String,
    #[arg(long)]
    pub(super) block: Option<String>,
}

#[derive(Parser, Debug)]
pub struct BalancerAddressArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) address: String,
}

#[derive(Parser, Debug)]
pub struct BalancerShardInfoArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[arg(long)]
    pub(super) workchain: i32,
    #[arg(long)]
    pub(super) shard: String,
    #[arg(long)]
    pub(super) exact: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerOneTransactionArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[arg(long)]
    pub(super) account: String,
    #[arg(long)]
    pub(super) lt: u64,
}

#[derive(Parser, Debug)]
pub struct BalancerTransactionsArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) account: String,
    #[arg(long)]
    pub(super) lt: u64,
    #[arg(long)]
    pub(super) hash: String,
    #[arg(long)]
    pub(super) count: u32,
}

#[derive(Parser, Debug)]
pub struct BalancerBlockTransactionsExtArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[arg(long)]
    pub(super) count: u32,
    #[arg(long)]
    pub(super) after_account: Option<String>,
    #[arg(long)]
    pub(super) after_lt: Option<u64>,
    #[arg(long)]
    pub(super) reverse_order: bool,
    #[arg(long)]
    pub(super) want_proof: bool,
}

#[derive(Parser, Debug)]
pub struct BalancerRunGetMethodArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) address: String,
    #[arg(long)]
    pub(super) block: Option<String>,
    #[arg(
        long,
        conflicts_with = "method_id",
        required_unless_present = "method_id"
    )]
    pub(super) method: Option<String>,
    #[arg(long, conflicts_with = "method", required_unless_present = "method")]
    pub(super) method_id: Option<u64>,
}

#[derive(Parser, Debug)]
pub struct BalancerConfigAllArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[command(flatten)]
    pub(super) flags: ConfigModeFlags,
}

#[derive(Parser, Debug)]
pub struct BalancerConfigParamsArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[arg(long)]
    pub(super) params: String,
    #[command(flatten)]
    pub(super) flags: ConfigModeFlags,
}

#[derive(Parser, Debug)]
pub struct BalancerLibrariesArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) libraries: String,
}

#[derive(Parser, Debug)]
pub struct BalancerLibrariesWithProofArgs {
    #[arg(short = 'n', long, default_value = "3")]
    pub(super) num_servers: usize,
    #[arg(long)]
    pub(super) block: String,
    #[arg(long)]
    pub(super) libraries: String,
    #[arg(long, default_value = "0")]
    pub(super) mode: u32,
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
    /// Run a get-method at the latest masterchain block.
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
        /// Stack argument: null, int:<decimal>, cell:<boc-hex>, slice:<boc-hex>, unsupported:<hex>, tuple:<json-array>, or list:<json-array>.
        #[arg(long = "arg", conflicts_with = "stack_json")]
        args: Vec<String>,
        /// Inline JSON stack array for scriptable nested input.
        #[arg(long, conflicts_with = "args")]
        stack_json: Option<String>,
    },
    /// Run a get-method using ABI JSON input and output metadata.
    RunAbiGetMethod(ContractAbiRunGetMethodArgs),
}

#[derive(Parser, Debug)]
pub struct ContractAbiRunGetMethodArgs {
    /// LiteServer index in the global config.
    #[arg(short = 'l', long, default_value = "0")]
    pub(super) ls_index: usize,
    /// Account address.
    #[arg(short = 'a', long)]
    pub(super) address: String,
    /// ABI JSON file.
    #[arg(long)]
    pub(super) abi_file: String,
    /// Contract name inside the ABI file. Required when the file has more than one contract.
    #[arg(long)]
    pub(super) contract: Option<String>,
    /// ABI get-method name. Required when the selected contract has more than one get-method.
    #[arg(short = 'm', long)]
    pub(super) method: Option<String>,
    /// ABI argument as name=json. Repeat for multiple arguments.
    #[arg(long = "arg")]
    pub(super) args: Vec<String>,
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
    pub(super) version: WalletVersionArg,
    /// Workchain used for derived address.
    #[arg(long, default_value = "0")]
    pub(super) workchain: i8,
    /// Override wallet id.
    #[arg(long)]
    pub(super) wallet_id: Option<u32>,
    /// Read mnemonic phrase from a file, or from stdin when set to '-'.
    #[arg(long, conflicts_with = "mnemonic_env")]
    pub(super) mnemonic_file: Option<String>,
    /// Read mnemonic phrase from an environment variable.
    #[arg(long, conflicts_with = "mnemonic_file")]
    pub(super) mnemonic_env: Option<String>,
    /// Read optional mnemonic password from this environment variable.
    #[arg(long)]
    pub(super) mnemonic_password_env: Option<String>,
}

#[derive(Parser, Debug)]
pub struct WalletTransferArgs {
    /// Wallet version to use.
    #[arg(long, default_value = "v5r1")]
    pub(super) version: WalletVersionArg,
    /// Destination address in raw or friendly form.
    #[arg(long)]
    pub(super) to: String,
    /// Amount in nanotons.
    #[arg(long)]
    pub(super) amount: u64,
    /// Optional text comment stored as a standard comment body.
    #[arg(long)]
    pub(super) comment: Option<String>,
    /// Send mode for the internal transfer.
    #[arg(long, default_value = "3")]
    pub(super) mode: u8,
    /// Message timeout in seconds. Used to compute valid_until from local time.
    #[arg(long, default_value = "60")]
    pub(super) timeout: u32,
    /// Override wallet seqno. Required for offline prepare-transfer.
    #[arg(long)]
    pub(super) seqno: Option<u32>,
    /// Override wallet id.
    #[arg(long)]
    pub(super) wallet_id: Option<u32>,
    /// Wallet workchain.
    #[arg(long, default_value = "0")]
    pub(super) workchain: i8,
    /// Include StateInit in the external message.
    #[arg(long)]
    pub(super) deploy: bool,
    /// Read mnemonic phrase from a file, or from stdin when set to '-'.
    #[arg(long, conflicts_with = "mnemonic_env")]
    pub(super) mnemonic_file: Option<String>,
    /// Read mnemonic phrase from an environment variable.
    #[arg(long, conflicts_with = "mnemonic_file")]
    pub(super) mnemonic_env: Option<String>,
    /// Read optional mnemonic password from this environment variable.
    #[arg(long)]
    pub(super) mnemonic_password_env: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BlockIdExtView {
    pub(super) workchain: i32,
    pub(super) shard: i64,
    pub(super) seqno: i32,
    pub(super) root_hash: String,
    pub(super) file_hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct MasterchainInfoView {
    pub(super) last: BlockIdExtView,
    pub(super) state_root_hash: String,
    pub(super) init_workchain: i32,
    pub(super) init_root_hash: String,
    pub(super) init_file_hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct VersionView {
    pub(super) mode: u32,
    pub(super) version: u32,
    pub(super) capabilities: u64,
    pub(super) now: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct TimeView {
    pub(super) now: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct RawBytesView {
    pub(super) hex: String,
    pub(super) base64: String,
    pub(super) len: usize,
}
