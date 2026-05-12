impl Cli {
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

    fn print_wallet_generate(&self, value: &WalletGenerateView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("mnemonic: {}", value.mnemonic);
                println!("public_key: {}", value.public_key);
                println!("v5r1_address: {}", value.v5r1.address);
                println!("v5r1_bounceable: {}", value.v5r1.bounceable);
                println!("v5r1_non_bounceable: {}", value.v5r1.non_bounceable);
                println!("v5r1_wallet_id: {}", value.v5r1.wallet_id);
                println!("v4r2_address: {}", value.v4r2.address);
                println!("v4r2_bounceable: {}", value.v4r2.bounceable);
                println!("v4r2_non_bounceable: {}", value.v4r2.non_bounceable);
                println!("v4r2_wallet_id: {}", value.v4r2.wallet_id);
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_wallet_address(&self, value: &WalletAddressView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("version: {:?}", value.version);
                println!("workchain: {}", value.workchain);
                println!("wallet_id: {}", value.wallet_id);
                println!("address: {}", value.address);
                println!("bounceable: {}", value.bounceable);
                println!("non_bounceable: {}", value.non_bounceable);
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_wallet_seqno(&self, value: &WalletSeqnoView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("address: {}", value.address);
                println!("seqno: {}", value.seqno);
                print_block_human("block", &value.block);
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_wallet_prepared(&self, value: &WalletPreparedTransferView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                println!("version: {:?}", value.version);
                println!("address: {}", value.address.address);
                println!("to: {}", value.to);
                println!("amount: {}", value.amount);
                println!("seqno: {}", value.seqno);
                println!("valid_until: {}", value.valid_until);
                println!("deploy: {}", value.deploy);
                println!("boc_hex: {}", value.boc.hex);
                println!("boc_base64: {}", value.boc.base64);
                Ok(())
            }
            _ => self.print_structured(value),
        }
    }

    fn print_wallet_send(&self, value: &WalletSendView) -> Result<()> {
        match self.output {
            OutputFormat::Human => {
                self.print_wallet_prepared(&value.prepared)?;
                println!("send_status: {}", value.status);
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
