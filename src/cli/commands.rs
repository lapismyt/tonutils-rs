use super::*;

pub(super) fn run_get_method_view(
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

pub(super) fn masterchain_info_view(
    info: crate::tl::response::MasterchainInfo,
) -> MasterchainInfoView {
    MasterchainInfoView {
        last: block_id_ext_view(&info.last),
        state_root_hash: info.state_root_hash.to_hex(),
        init_workchain: info.init.workchain,
        init_root_hash: info.init.root_hash.to_hex(),
        init_file_hash: info.init.file_hash.to_hex(),
    }
}

pub(super) fn network_name(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
    }
}

pub(super) enum HighLevelBackend {
    Single(LiteClient),
    Balanced(LiteBalancer),
}

impl HighLevelBackend {
    pub(super) fn view(&self, cli: &Cli) -> BackendView {
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

    pub(super) async fn close(self) -> Result<()> {
        match self {
            HighLevelBackend::Single(_) => Ok(()),
            HighLevelBackend::Balanced(mut balancer) => {
                balancer.close_all().await?;
                Ok(())
            }
        }
    }

    pub(super) async fn get_masterchain_info(
        &mut self,
    ) -> Result<crate::tl::response::MasterchainInfo> {
        match self {
            HighLevelBackend::Single(client) => Ok(client.get_masterchain_info().await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer.get_masterchain_info().await?),
        }
    }

    pub(super) async fn get_account_state(
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

    pub(super) async fn raw_get_block_data(
        &mut self,
        block: BlockIdExt,
    ) -> Result<crate::liteclient::boc::DecodedBlockData> {
        match self {
            HighLevelBackend::Single(client) => Ok(client.raw_get_block_data(block).await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer.raw_get_block_data(block).await?),
        }
    }

    pub(super) async fn run_get_method(
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

    pub(super) async fn raw_get_transactions(
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

    pub(super) async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32> {
        match self {
            HighLevelBackend::Single(client) => Ok(client.send_message(body).await?),
            HighLevelBackend::Balanced(balancer) => Ok(balancer.send_message(body).await?),
        }
    }

    pub(super) async fn get_config_all_typed(
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

    pub(super) async fn get_config_params_typed(
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

pub(super) fn best_effort_account_state_view(
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

    let decoded = match crate::tvm::Address::from_str(address) {
        Ok(address) => {
            crate::liteclient::boc::DecodedAccountState::from_raw_verified(raw.clone(), &address)
                .ok()
        }
        Err(error) => {
            decode_errors.push(format!(
                "address parse failed for proof extraction: {error}"
            ));
            None
        }
    };
    let account = decoded
        .as_ref()
        .and_then(|decoded| decoded.account.clone())
        .or_else(|| {
            if raw.state.is_empty() {
                None
            } else {
                match crate::liteclient::boc::decode_account_state_boc(&raw.state) {
                    Ok(value) => Some(value.account),
                    Err(error) => {
                        decode_errors.push(format!("state TL-B decode failed: {error}"));
                        None
                    }
                }
            }
        });
    let shard_account = decoded
        .as_ref()
        .and_then(|decoded| decoded.shard_account.clone());
    let (state, balance) = account_summary(account.as_ref());
    let last_transaction_lt = shard_account
        .as_ref()
        .map(|account| account.last_trans_lt)
        .or_else(|| {
            account.as_ref().and_then(|account| match account {
                crate::tlb::Account::None => None,
                crate::tlb::Account::Full { storage, .. } => Some(storage.last_trans_lt),
            })
        });
    let last_transaction_hash = shard_account
        .as_ref()
        .map(|account| hex::encode(account.last_trans_hash));

    BestEffortAccountStateView {
        address: address.to_owned(),
        block: block_id_ext_view(&raw.id),
        shard_block: block_id_ext_view(&raw.shardblk),
        state,
        balance,
        last_transaction_lt,
        last_transaction_hash,
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
        shard_account: shard_account.as_ref().map(shard_account_value),
        decode_errors,
    }
}

pub(super) fn decode_root_hashes(
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

pub(super) fn decode_root_hash(
    raw: &[u8],
    label: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
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

pub(super) fn account_summary(account: Option<&crate::tlb::Account>) -> (String, Option<String>) {
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

pub(super) fn wallet_id_for_cli(
    version: WalletVersionArg,
    network: Network,
    workchain: i8,
    wallet_id: Option<u32>,
) -> Result<u32> {
    if let Some(wallet_id) = wallet_id {
        return Ok(wallet_id);
    }
    match version {
        WalletVersionArg::V4R2 => Ok(WALLET_V4R2_DEFAULT_ID),
        WalletVersionArg::V5R1 => {
            let global_id = match network {
                Network::Mainnet => MAINNET_GLOBAL_ID,
                Network::Testnet => TESTNET_GLOBAL_ID,
            };
            Ok(WalletV5R1WalletId::client(global_id, workchain, 0, 0).pack()?)
        }
    }
}

pub(super) fn wallet_address_view(
    version: WalletVersionArg,
    workchain: i8,
    wallet_id: u32,
    public_key: [u8; 32],
) -> Result<WalletAddressView> {
    let address = match version {
        WalletVersionArg::V4R2 => {
            WalletV4R2::new(public_key, wallet_id, wallet_v4r2_code()?, workchain).address()?
        }
        WalletVersionArg::V5R1 => {
            WalletV5R1::new(public_key, wallet_id, wallet_v5r1_code()?, workchain).address()?
        }
    };
    Ok(WalletAddressView {
        version,
        workchain,
        wallet_id,
        address: address.to_raw(),
        bounceable: address.to_bounceable(true),
        non_bounceable: address.to_non_bounceable(true),
    })
}

pub(super) fn read_mnemonic_phrase(file: &Option<String>, env: &Option<String>) -> Result<String> {
    match (file, env) {
        (Some(path), None) if path == "-" => {
            let mut phrase = String::new();
            io::stdin().read_to_string(&mut phrase)?;
            Ok(phrase)
        }
        (Some(path), None) => {
            fs::read_to_string(path).with_context(|| format!("failed to read mnemonic file {path}"))
        }
        (None, Some(name)) => std::env::var(name)
            .with_context(|| format!("failed to read mnemonic from environment variable {name}")),
        (None, None) => {
            let mut phrase = String::new();
            io::stdin().read_to_string(&mut phrase)?;
            Ok(phrase)
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("--mnemonic-file and --mnemonic-env are mutually exclusive")
        }
    }
}

pub(super) fn read_mnemonic_password(env: &Option<String>) -> Result<Option<String>> {
    env.as_ref()
        .map(|name| {
            std::env::var(name).with_context(|| {
                format!("failed to read mnemonic password from environment variable {name}")
            })
        })
        .transpose()
}

pub(super) fn comment_body(comment: &Option<String>) -> Result<Option<Arc<Cell>>> {
    let Some(comment) = comment else {
        return Ok(None);
    };
    let mut builder = Builder::new();
    builder.store_u32(0)?;
    builder.store_bytes(comment.as_bytes())?;
    Ok(Some(builder.build()?))
}

pub(super) fn valid_until_from_timeout(timeout: u32) -> Result<u32> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_secs();
    let valid_until = now
        .checked_add(timeout as u64)
        .context("valid_until overflow")?;
    u32::try_from(valid_until).context("valid_until does not fit into uint32")
}

pub(super) fn seqno_from_stack(result: crate::tl::response::RunMethodResult) -> Result<u32> {
    let stack = match result.result_stack_lossless() {
        DecodedRunMethodResult::Decoded(stack) => stack,
        DecodedRunMethodResult::Missing => anyhow::bail!("seqno get-method returned no stack"),
        DecodedRunMethodResult::Undecodable { error, .. } => {
            anyhow::bail!("failed to decode seqno stack: {error}")
        }
    };
    let Some(TvmStackEntry::Int(value)) = stack.entries().first() else {
        anyhow::bail!("seqno get-method did not return an integer at stack[0]");
    };
    value
        .to_str_radix(10)
        .parse::<u32>()
        .context("seqno integer does not fit into uint32")
}

pub(super) fn seqno_from_stack_or_deploy_zero(
    result: crate::tl::response::RunMethodResult,
    deploy: bool,
) -> Result<u32> {
    if deploy
        && matches!(
            result.result_stack_lossless(),
            DecodedRunMethodResult::Missing
        )
    {
        return Ok(0);
    }
    seqno_from_stack(result)
}

pub(super) fn build_wallet_transfer(
    network: Network,
    args: &WalletTransferArgs,
    mnemonic: &TonMnemonic,
    seqno: u32,
) -> Result<(Vec<u8>, WalletPreparedTransferView)> {
    let public_key = mnemonic.public_key();
    let wallet_id = wallet_id_for_cli(args.version, network, args.workchain, args.wallet_id)?;
    let address = wallet_address_view(args.version, args.workchain, wallet_id, public_key)?;
    let mut message = WalletMessage::internal(
        Address::from_str(&args.to).context("wallet transfer: invalid destination address")?,
        args.amount,
    )
    .with_mode(args.mode);
    if let Some(body) = comment_body(&args.comment)? {
        message = message.with_body(body);
    }
    let valid_until = valid_until_from_timeout(args.timeout)?;
    let boc = match args.version {
        WalletVersionArg::V4R2 => {
            WalletV4R2::new(public_key, wallet_id, wallet_v4r2_code()?, args.workchain)
                .build_external_message_boc(
                    seqno,
                    valid_until,
                    vec![message],
                    mnemonic.signing_key(),
                    args.deploy,
                )?
        }
        WalletVersionArg::V5R1 => {
            WalletV5R1::new(public_key, wallet_id, wallet_v5r1_code()?, args.workchain)
                .build_external_message_boc(
                    seqno,
                    valid_until,
                    vec![message],
                    mnemonic.signing_key(),
                    args.deploy,
                )?
        }
    };
    let view = WalletPreparedTransferView {
        version: args.version,
        address,
        to: args.to.clone(),
        amount: args.amount,
        seqno,
        valid_until,
        deploy: args.deploy,
        boc: raw_bytes_view(&boc),
    };
    Ok((boc, view))
}

pub(super) async fn download_config(network: Network) -> Result<String> {
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
