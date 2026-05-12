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

#[derive(Debug, Serialize)]
struct WalletAddressView {
    version: WalletVersionArg,
    workchain: i8,
    wallet_id: u32,
    address: String,
    bounceable: String,
    non_bounceable: String,
}

#[derive(Debug, Serialize)]
struct WalletGenerateView {
    mnemonic: String,
    public_key: String,
    v5r1: WalletAddressView,
    v4r2: WalletAddressView,
}

#[derive(Debug, Serialize)]
struct WalletSeqnoView {
    address: String,
    seqno: u32,
    block: BlockIdExtView,
}

#[derive(Debug, Serialize)]
struct WalletPreparedTransferView {
    version: WalletVersionArg,
    address: WalletAddressView,
    to: String,
    amount: u64,
    seqno: u32,
    valid_until: u32,
    deploy: bool,
    boc: RawBytesView,
}

#[derive(Debug, Serialize)]
struct WalletSendView {
    prepared: WalletPreparedTransferView,
    status: u32,
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

