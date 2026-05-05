use crate::contracts::{Contract, DecodedRunMethodResult, RunMethodResultExt};
use crate::liteclient::{balancer::LiteBalancer, client::LiteClient, rate_limit::RequestRateLimit};
use crate::network_config::ConfigGlobal;
use crate::tl::BlockIdExt;
use crate::tvm::{TvmStack, TvmStackEntry, address::Address};
use anyhow::{Context, Result};
use base64::Engine;
use clap::{Parser, Subcommand, ValueEnum};
use num_bigint::BigInt;
use serde::Serialize;
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
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// LiteServer requests through one LiteClient.
    Liteclient {
        #[command(subcommand)]
        command: LiteClientCommand,
    },
    /// LiteServer requests through LiteBalancer.
    Balancer {
        #[command(subcommand)]
        command: BalancerCommand,
    },
    /// High-level smart-contract helpers over LiteClient.
    Contract {
        #[command(subcommand)]
        command: ContractCommand,
    },
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

    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            Commands::Liteclient { command } => self.execute_liteclient(command).await,
            Commands::Balancer { command } => self.execute_balancer(command).await,
            Commands::Contract { command } => self.execute_contract(command).await,
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
        }
    }

    fn print_structured<T: Serialize + std::fmt::Debug>(&self, value: &T) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("{value:#?}");
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

fn read_raw_input(
    hex_input: &Option<String>,
    base64_input: &Option<String>,
    file: &Option<String>,
    stdin: bool,
) -> Result<Vec<u8>> {
    match (hex_input, base64_input, file, stdin) {
        (Some(value), None, None, false) => {
            hex::decode(value.trim()).context("failed to decode hex request")
        }
        (None, Some(value), None, false) => base64::engine::general_purpose::STANDARD
            .decode(value.trim())
            .context("failed to decode base64 request"),
        (None, None, Some(path), false) => {
            fs::read(path).with_context(|| format!("failed to read request file {path}"))
        }
        (None, None, None, true) => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        (None, None, None, false) => {
            anyhow::bail!("raw-query requires one of --hex, --base64, --file, or --stdin")
        }
        _ => anyhow::bail!("raw-query accepts only one input source"),
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
