use tonutils_liteclient::{
    boc::{DecodedAccountState, DecodedBocRoots},
    client::LiteClient,
    method_name_to_id,
};
use tonutils_network_config::ConfigGlobal;
use tonutils_tl::Int256;
use tonutils_tlb::{TlbDeserialize, TlbSerialize};
use tonutils_tvm::{Address, TvmStack};

fn load_config() -> Option<ConfigGlobal> {
    let json = match std::env::var("TON_GLOBAL_CONFIG_JSON") {
        Ok(json) => json,
        Err(std::env::VarError::NotPresent) => {
            eprintln!("skipping live smoke test: TON_GLOBAL_CONFIG_JSON is not set");
            return None;
        }
        Err(error) => panic!("failed to read TON_GLOBAL_CONFIG_JSON: {error}"),
    };

    Some(
        json.parse()
            .expect("TON_GLOBAL_CONFIG_JSON must be valid JSON"),
    )
}

async fn connect() -> Option<LiteClient> {
    let config = load_config()?;
    let index = std::env::var("TON_LS_INDEX")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    match LiteClient::connect_config(&config, index).await {
        Ok(client) => Some(client),
        Err(error) => panic!("live liteserver connection failed: {error}"),
    }
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_get_masterchain_info_smoke() {
    let Some(mut client) = connect().await else {
        return;
    };
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return a typed response");
    assert!(info.last.seqno > 0);
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_get_version_and_time_smoke() {
    let Some(mut client) = connect().await else {
        return;
    };
    let version = client
        .get_version()
        .await
        .expect("getVersion must return a typed response");
    let now = client
        .get_time()
        .await
        .expect("getTime must return a timestamp");
    assert!(version.version > 0);
    assert!(now > 0);
}

#[tokio::test]
#[ignore = "requires public liteserver configuration, network access, and a contract"]
#[allow(clippy::assert_is_empty)]
async fn live_run_get_method_smoke() {
    let address = match std::env::var("TON_CONTRACT_ADDRESS") {
        Ok(address) => address,
        Err(std::env::VarError::NotPresent) => {
            eprintln!(
                "skipping live get-method smoke test: TON_CONTRACT_ADDRESS is not set; \
                 provide a contract address to run seqno"
            );
            return;
        }
        Err(error) => panic!("failed to read TON_CONTRACT_ADDRESS: {error}"),
    };
    let address =
        Address::from_str(&address).expect("TON_CONTRACT_ADDRESS must be a valid address");
    let Some(mut client) = connect().await else {
        return;
    };
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return a typed response");
    let stack = client
        .run_get_method_typed(
            tonutils_contracts::RUN_METHOD_MODE_RETURN_RESULT,
            info.last,
            address,
            method_name_to_id("seqno"),
            TvmStack::empty(),
        )
        .await
        .expect("run_get_method(seqno) must return a successful stack");
    assert!(!stack.is_empty());
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_list_latest_masterchain_transactions_batch() {
    let Some(mut client) = connect().await else {
        return;
    };
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return the latest masterchain block");
    let response = client
        .list_block_transactions_ext_decoded(info.last.clone(), 256, None, false, false)
        .await
        .expect("listBlockTransactionsExt must return the latest masterchain transactions");

    assert_eq!(response.raw.id, info.last);
    assert!(response.raw.req_count > 0);
    assert!(
        !response.raw.incomplete,
        "latest masterchain block transaction list is incomplete"
    );
    assert!(
        !response.transactions.is_empty(),
        "latest masterchain block returned no decoded transactions"
    );

    let roots = DecodedBocRoots::decode(&response.raw.transactions)
        .expect("the transaction batch must be a decodable BoC");
    assert_eq!(roots.roots.len(), response.transactions.len());
    for (root, transaction) in roots.roots.into_iter().zip(response.transactions) {
        let decoded = tonutils_tlb::Transaction::from_cell(root)
            .expect("every transaction root in the common BoC must decode");
        assert_eq!(decoded, transaction);
    }
}

#[tokio::test]
#[ignore = "requires public liteserver configuration, network access, and a contract"]
async fn live_get_twelve_contract_transactions_in_four_batches() {
    let Some(mut client) = connect().await else {
        return;
    };
    let address = match std::env::var("TON_CONTRACT_ADDRESS") {
        Ok(address) => address,
        Err(std::env::VarError::NotPresent) => {
            eprintln!(
                "skipping live transaction history test: TON_CONTRACT_ADDRESS is not set; \
                 provide a contract address to inspect transaction history"
            );
            return;
        }
        Err(error) => panic!("failed to read TON_CONTRACT_ADDRESS: {error}"),
    };
    let address = Address::from_str(&address).expect("TON_CONTRACT_ADDRESS must be valid");
    let account = address.to_account_id();
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return the latest masterchain block");
    let raw_state = client
        .get_account_state(info.last, account.clone())
        .await
        .expect("getAccountState must return the configured contract state");
    let state = DecodedAccountState::from_raw_verified(raw_state, &address)
        .expect("configured contract account state must contain a verifiable shard account")
        .simple();
    let mut cursor_lt = state
        .last_transaction_lt
        .expect("configured contract has no transaction history");
    let mut cursor_hash = state
        .last_transaction_hash
        .expect("configured contract state does not expose the latest transaction hash");
    let mut seen = Vec::with_capacity(12);

    for page in 0..4 {
        let (transactions, ids) = client
            .raw_get_transactions(3, account.clone(), cursor_lt, Int256(cursor_hash))
            .await
            .unwrap_or_else(|error| panic!("getTransactions page {} failed: {error}", page + 1));
        assert_eq!(
            ids.len(),
            3,
            "getTransactions page {} returned incomplete ids",
            page + 1
        );
        assert_eq!(
            transactions.len(),
            3,
            "getTransactions page {} did not decode exactly three transactions",
            page + 1
        );

        let mut previous_lt = cursor_lt;
        for transaction in &transactions {
            assert_eq!(transaction.account_addr, address.hash_part);
            assert!(
                transaction.lt <= previous_lt,
                "transaction logical times must move toward older history"
            );
            previous_lt = transaction.lt;
        }

        let last = transactions
            .last()
            .expect("validated transaction page must contain a last transaction");
        let next_lt = last.lt;
        let next_hash = last
            .to_cell()
            .expect("decoded transaction must serialize for cursor progression")
            .hash();
        assert!(
            next_lt < cursor_lt,
            "transaction cursor did not move toward older history on page {}",
            page + 1
        );
        cursor_lt = next_lt;
        cursor_hash = next_hash;
        seen.extend(transactions);
    }

    assert_eq!(seen.len(), 12);
}
