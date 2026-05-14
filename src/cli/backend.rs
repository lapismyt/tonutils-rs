use super::*;

impl Cli {
    pub fn parse_args() -> Self {
        Cli::parse()
    }

    pub(super) async fn load_config(&self) -> Result<ConfigGlobal> {
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

    pub(super) async fn create_balancer(&self, num_servers: usize) -> Result<LiteBalancer> {
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

    pub(super) async fn create_high_level_backend(&self) -> Result<HighLevelBackend> {
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
            Commands::Wallet { command } => self.execute_wallet(command).await,
            Commands::Tvm { command } => self.execute_tvm(command).await,
        }
    }

    pub(super) async fn execute_status(&self) -> Result<()> {
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

    pub(super) async fn execute_account(&self, args: &HighLevelAccountArgs) -> Result<()> {
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

    pub(super) async fn execute_call(&self, args: &HighLevelCallArgs) -> Result<()> {
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

    pub(super) async fn execute_transactions(
        &self,
        args: &HighLevelTransactionsArgs,
    ) -> Result<()> {
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

    pub(super) async fn execute_block(&self, command: &BlockCommand) -> Result<()> {
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

    pub(super) async fn execute_config(&self, command: &ConfigCommand) -> Result<()> {
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

    pub(super) async fn execute_tvm(&self, command: &TvmCommand) -> Result<()> {
        match command {
            TvmCommand::Boc { command } => self.execute_boc(command),
            TvmCommand::Schema { command } => self.execute_schema(command),
        }
    }

    pub(super) fn execute_schema(&self, command: &SchemaCommand) -> Result<()> {
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

    pub(super) fn execute_boc(&self, command: &BocCommand) -> Result<()> {
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

    pub(super) async fn execute_liteclient(&self, command: &LiteClientCommand) -> Result<()> {
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

    pub(super) async fn execute_contract(&self, command: &ContractCommand) -> Result<()> {
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
            ContractCommand::RunAbiGetMethod(args) => {
                let definition = load_abi_file(&args.abi_file)?;
                let abi_contract = select_abi_contract(&definition, args.contract.as_deref())?;
                let function = select_abi_get_method(abi_contract, args.method.as_deref())?;
                let method_id = abi_get_method_id(function)?;
                let stack = TvmStack::new(encode_abi_get_method_inputs(function, &args.args)?);
                let mut client = self.create_client(args.ls_index).await?;
                let address = Address::from_str(&args.address)?;
                let mut contract = Contract::new(&mut client, address);
                let result = contract.run_get_method_latest(method_id, stack).await?;
                self.print_structured(&abi_get_method_view(
                    result,
                    abi_contract,
                    function,
                    method_id,
                )?)
            }
        }
    }

    pub(super) async fn execute_wallet(&self, command: &WalletCommand) -> Result<()> {
        match command {
            WalletCommand::Generate {
                workchain,
                mnemonic_password_env,
            } => {
                let password = read_mnemonic_password(mnemonic_password_env)?;
                let mnemonic = TonMnemonic::generate(password.as_deref())?;
                let public_key = mnemonic.public_key();
                let v5_wallet_id =
                    wallet_id_for_cli(WalletVersionArg::V5R1, self.network, *workchain, None)?;
                let v4_wallet_id =
                    wallet_id_for_cli(WalletVersionArg::V4R2, self.network, *workchain, None)?;
                self.print_wallet_generate(&WalletGenerateView {
                    mnemonic: mnemonic.phrase(),
                    public_key: hex::encode(public_key),
                    v5r1: wallet_address_view(
                        WalletVersionArg::V5R1,
                        *workchain,
                        v5_wallet_id,
                        public_key,
                    )?,
                    v4r2: wallet_address_view(
                        WalletVersionArg::V4R2,
                        *workchain,
                        v4_wallet_id,
                        public_key,
                    )?,
                })
            }
            WalletCommand::Address(args) => {
                let password = read_mnemonic_password(&args.mnemonic_password_env)?;
                let phrase = read_mnemonic_phrase(&args.mnemonic_file, &args.mnemonic_env)?;
                let mnemonic = TonMnemonic::from_phrase(&phrase, password.as_deref())?;
                let wallet_id =
                    wallet_id_for_cli(args.version, self.network, args.workchain, args.wallet_id)?;
                self.print_wallet_address(&wallet_address_view(
                    args.version,
                    args.workchain,
                    wallet_id,
                    mnemonic.public_key(),
                )?)
            }
            WalletCommand::Seqno { address } => {
                let mut backend = self.create_high_level_backend().await?;
                let latest = backend
                    .get_masterchain_info()
                    .await
                    .context("wallet seqno: failed to get latest block")?
                    .last;
                let result = backend
                    .run_get_method(
                        latest.clone(),
                        Address::from_str(address).context("wallet seqno: invalid address")?,
                        crate::utils::method_name_to_id("seqno"),
                        TvmStack::empty(),
                    )
                    .await
                    .context("wallet seqno: get-method failed")?;
                backend.close().await?;
                self.print_wallet_seqno(&WalletSeqnoView {
                    address: address.clone(),
                    seqno: seqno_from_stack(result)?,
                    block: block_id_ext_view(&latest),
                })
            }
            WalletCommand::PrepareTransfer(args) => {
                let seqno = args
                    .seqno
                    .context("wallet prepare-transfer requires --seqno for offline signing")?;
                let password = read_mnemonic_password(&args.mnemonic_password_env)?;
                let phrase = read_mnemonic_phrase(&args.mnemonic_file, &args.mnemonic_env)?;
                let mnemonic = TonMnemonic::from_phrase(&phrase, password.as_deref())?;
                let (boc, view) = build_wallet_transfer(self.network, args, &mnemonic, seqno)?;
                match self.output {
                    OutputFormat::Raw | OutputFormat::Hex | OutputFormat::Base64 => {
                        self.print_bytes(&boc)
                    }
                    OutputFormat::Human => self.print_wallet_prepared(&view),
                    OutputFormat::Json | OutputFormat::PrettyJson => self.print_structured(&view),
                }
            }
            WalletCommand::Send(args) => {
                let password = read_mnemonic_password(&args.mnemonic_password_env)?;
                let phrase = read_mnemonic_phrase(&args.mnemonic_file, &args.mnemonic_env)?;
                let mnemonic = TonMnemonic::from_phrase(&phrase, password.as_deref())?;
                let public_key = mnemonic.public_key();
                let wallet_id =
                    wallet_id_for_cli(args.version, self.network, args.workchain, args.wallet_id)?;
                let wallet_address =
                    wallet_address_view(args.version, args.workchain, wallet_id, public_key)?;
                let mut backend = self.create_high_level_backend().await?;
                let latest = backend
                    .get_masterchain_info()
                    .await
                    .context("wallet send: failed to get latest block")?
                    .last;
                let seqno = match args.seqno {
                    Some(seqno) => seqno,
                    None => {
                        let result = backend
                            .run_get_method(
                                latest,
                                Address::from_str(&wallet_address.address)?,
                                crate::utils::method_name_to_id("seqno"),
                                TvmStack::empty(),
                            )
                            .await
                            .context("wallet send: failed to fetch seqno")?;
                        seqno_from_stack_or_deploy_zero(result, args.deploy)?
                    }
                };
                let (boc, view) = build_wallet_transfer(self.network, args, &mnemonic, seqno)?;
                let status = backend
                    .send_external_message_boc(boc)
                    .await
                    .context("wallet send: failed to submit external message BoC")?;
                backend.close().await?;
                self.print_wallet_send(&WalletSendView {
                    prepared: view,
                    status,
                })
            }
        }
    }
}
