use clap::{Parser, Subcommand};
use rand::seq::SliceRandom;
use crate::liteclient::{client::LiteClient, balancer::LiteBalancer};
use crate::network_config::ConfigGlobal;
use crate::tl::BlockIdExt;
use crate::utils::method_name_to_id;
use crate::tvm::address::Address;
use anyhow::{Result};
use std::str::FromStr;
use std::time::{Duration, Instant};

/// tonutils-rs CLI
#[derive(Parser, Debug)]
#[command(name = "tonutils-rs")]
#[command(about = "tonutils-rs CLI", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Get masterchain info
    GetMasterchainInfo {
        /// LiteServer index
        #[arg(short = 'l', long)]
        ls_index: u8,
    },
    /// Get masterchain info using LiteBalancer (load balanced across multiple liteservers)
    GetMasterchainInfoBalanced {
        /// Number of liteservers to use (default: 3)
        #[arg(short = 'n', long, default_value = "3")]
        num_servers: usize,
    },
    /// Run smart contract method
    RunSmcMethod {
        /// LiteServer index
        #[arg(short = 'l', long)]
        ls_index: u8,
        /// Account address
        #[arg(short = 'a', long)]
        address: String,
        /// Method name
        #[arg(short = 'm', long)]
        method: String,
    }
}

async fn download_config(testnet: bool) -> Result<String> {
    let url = if testnet {
        "https://ton.org/testnet-global.config.json"
    } else {
        "https://ton.org/global.config.json"
    };
    let mut response = ureq::get(url).call()
        .map_err(|e| anyhow::anyhow!("Error occurred while fetching config from {}: {:?}.", url, e))?;
    if response.status() != 200 {
        return Err(anyhow::anyhow!(
            "Url {} responded with error code {}",
            url,
            response.status()
        ));
    }
    Ok(response.body_mut().read_to_string()?)
}

impl Cli {
    /// Parse command line arguments
    pub fn parse_args() -> Self {
        Cli::parse()
    }

    pub async fn create_client(&self, ls_index: Option<u8>) -> Result<LiteClient> {
        let config_json = download_config(false).await?;
        let config: ConfigGlobal = ConfigGlobal::from_str(&config_json)?;

        let ls = match ls_index {
            Some(ls_index) => &config.liteservers[ls_index as usize],
            None => config.liteservers.choose(&mut rand::thread_rng()).unwrap(),
        };
        let public_key: [u8; 32] = ls.id.clone().into();
        

        let client = LiteClient::connect(ls.socket_addr(), public_key).await?;
        
        Ok(client)
    }

    /// Execute the command
    pub async fn execute(&self) -> Result<()> {
        
        match &self.command {
            Commands::GetMasterchainInfo { ls_index } => {
                self.execute_get_masterchain_info(*ls_index).await
            }
            Commands::GetMasterchainInfoBalanced { num_servers } => {
                self.execute_get_masterchain_info_balanced(*num_servers).await
            },
            Commands::RunSmcMethod { ls_index, address, method } => {
                self.execute_run_smc_method(*ls_index, &address, &method).await
            },
        }
    }

    /// Get masterchain info
    async fn execute_get_masterchain_info(&self, ls_index: u8) -> Result<()> {
        // Measure client initialization time
        let init_start = Instant::now();
        let mut client = self.create_client(Some(ls_index)).await?;
        let init_duration = init_start.elapsed();
        log::info!("⏱️  Client initialization: {:.3}s", init_duration.as_secs_f64());
        
        // Measure get_version operation
        let op_start = Instant::now();
        let version = client.get_version().await?;
        let op_duration = op_start.elapsed();
        log::info!("LiteServer mode {}", version.mode);
        log::info!("LiteServer version {}", version.version);
        log::info!("⏱️  get_version: {:.3}s", op_duration.as_secs_f64());
        
        // Measure get_masterchain_info operation
        let op_start = Instant::now();
        let info = client.get_masterchain_info().await?;
        let op_duration = op_start.elapsed();
        log::info!("Masterchain info: {:?}", info);
        log::info!("⏱️  get_masterchain_info: {:.3}s", op_duration.as_secs_f64());
        
        Ok(())
    }

    /// Get masterchain info using LiteBalancer
    async fn execute_get_masterchain_info_balanced(&self, num_servers: usize) -> Result<()> {
        log::info!("Creating LiteBalancer with {} liteservers", num_servers);
        
        // Measure total balancer initialization time
        let total_init_start = Instant::now();
        
        // Download config
        let config_json = download_config(false).await?;
        let config: ConfigGlobal = ConfigGlobal::from_str(&config_json)?;
        
        // Create multiple clients
        let mut clients = Vec::new();
        let num_to_use = num_servers.min(config.liteservers.len());
        
        log::info!("Connecting to {} liteservers...", num_to_use);
        for i in 0..num_to_use {
            let ls = &config.liteservers[i];
            let public_key: [u8; 32] = ls.id.clone().into();
            
            let connect_start = Instant::now();
            let connect_result = LiteClient::connect(ls.socket_addr(), public_key).await;
            match connect_result {
                Ok(client) => {
                    let connect_duration = connect_start.elapsed();
                    log::info!("✓ Connected to liteserver #{} ({}) in {:.3}s", 
                        i, ls.socket_addr(), connect_duration.as_secs_f64());
                    clients.push(client);
                }
                Err(e) => {
                    let connect_duration = connect_start.elapsed();
                    log::warn!("✗ Failed to connect to liteserver #{} after {:.3}s: {}", 
                        i, connect_duration.as_secs_f64(), e);
                }
            }
        }
        
        if clients.is_empty() {
            return Err(anyhow::anyhow!("Failed to connect to any liteservers"));
        }
        
        // Create and start up balancer
        let startup_start = Instant::now();
        let mut balancer = LiteBalancer::new(clients, Duration::from_secs(10));
        balancer.start_up().await?;
        let startup_duration = startup_start.elapsed();
        
        let total_init_duration = total_init_start.elapsed();
        log::info!("✓ LiteBalancer initialized");
        log::info!("  Total peers: {}", balancer.peers_num());
        log::info!("  Alive peers: {}", balancer.alive_peers_num().await);
        log::info!("  Archival peers: {}", balancer.archival_peers_num().await);
        log::info!("⏱️  Balancer startup: {:.3}s", startup_duration.as_secs_f64());
        log::info!("⏱️  Total initialization: {:.3}s", total_init_duration.as_secs_f64());
        
        // Get version from balancer
        let op_start = Instant::now();
        match balancer.get_version().await {
            Ok(version) => {
                let op_duration = op_start.elapsed();
                log::info!("LiteServer mode: {}", version.mode);
                log::info!("LiteServer version: {}", version.version);
                log::info!("LiteServer capabilities: {}", version.capabilities);
                log::info!("⏱️  get_version: {:.3}s", op_duration.as_secs_f64());
            }
            Err(e) => {
                let op_duration = op_start.elapsed();
                log::warn!("Failed to get version after {:.3}s: {}", op_duration.as_secs_f64(), e);
            }
        }
        
        // Get time from balancer
        let op_start = Instant::now();
        match balancer.get_time().await {
            Ok(time) => {
                let op_duration = op_start.elapsed();
                log::info!("LiteServer time: {}", time);
                log::info!("⏱️  get_time: {:.3}s", op_duration.as_secs_f64());
            }
            Err(e) => {
                let op_duration = op_start.elapsed();
                log::warn!("Failed to get time after {:.3}s: {}", op_duration.as_secs_f64(), e);
            }
        }
        
        // Get masterchain info using balancer
        log::info!("Fetching masterchain info through balancer...");
        let op_start = Instant::now();
        let info = balancer.get_masterchain_info().await?;
        let op_duration = op_start.elapsed();
        
        log::info!("✓ Masterchain info retrieved successfully:");
        log::info!("  Last block:");
        log::info!("    Workchain: {}", info.last.workchain);
        log::info!("    Shard: {}", info.last.shard);
        log::info!("    Seqno: {}", info.last.seqno);
        log::info!("    Root hash: {:?}", info.last.root_hash);
        log::info!("    File hash: {:?}", info.last.file_hash);
        log::info!("  State root hash: {:?}", info.state_root_hash);
        log::info!("  Init block:");
        log::info!("    Workchain: {}", info.init.workchain);
        log::info!("    Root hash: {:?}", info.init.root_hash);
        log::info!("    File hash: {:?}", info.init.file_hash);
        log::info!("⏱️  get_masterchain_info: {:.3}s", op_duration.as_secs_f64());
        
        // Clean shutdown
        let close_start = Instant::now();
        balancer.close_all().await?;
        let close_duration = close_start.elapsed();
        log::info!("✓ LiteBalancer closed successfully");
        log::info!("⏱️  close_all: {:.3}s", close_duration.as_secs_f64());
        
        Ok(())
    }

    async fn execute_run_smc_method(&self, ls_index: u8, address: &str, method: &str) -> Result<()> {
        // Measure client initialization time
        let init_start = Instant::now();
        let mut client = self.create_client(Some(ls_index)).await?;
        let init_duration = init_start.elapsed();
        log::info!("⏱️  Client initialization: {:.3}s", init_duration.as_secs_f64());

        let method_id = method_name_to_id(method);
        let address = Address::from_str(address)?;
        let stack = vec![];
        let init_duration = init_start.elapsed();
        let last_block = client.get_masterchain_info().await?.last;
        log::info!("⏱️  get_masterchain_info: {:.3}s", init_duration.as_secs_f64());
        let block: BlockIdExt = BlockIdExt {
            workchain: 0,
            shard: last_block.shard,
            seqno: last_block.seqno,
            root_hash: last_block.root_hash,
            file_hash: last_block.file_hash,
        };
        let init_duration = init_start.elapsed();
        let result = client.run_smc_method(0, block, address, method_id, stack).await?;
        log::info!("⏱️  run_smc_method: {:.3}s", init_duration.as_secs_f64());
        log::info!("result: {:?}", result);
        Ok(())
    }
}
