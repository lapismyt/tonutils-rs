#![cfg(feature = "provider")]

//! Live mainnet probes for the HTTP provider adapters.
//!
//! Toncenter allows one request per second without an API key, so every HTTP
//! call waits for a request slot and rate-limit responses are retried with
//! backoff. The probes only read chain state; they never submit external
//! messages.

mod provider_live_common;

use std::time::Instant;

use anyhow::{Context, Result, bail};
use provider_live_common::{
    PROVIDER_TIMEOUT, elapsed_millis, is_retryable, request_slot, retry_delay,
};
use tonutils::{
    provider::{
        AccountState, BlockId, GetAccountStateExt, GetMasterchainInfoExt, MethodRef, Provider,
        ProviderError, ProviderResult, RunGetMethodExt, TonApi, ToncenterV2, ToncenterV3,
        TransactionList,
    },
    tl::TlInt256 as Int256,
    tlb::AccountStatus,
    tvm::{Address, TvmStack, TvmStackEntry},
};

/// USDT jetton master on mainnet. It is an always-active contract with a
/// no-argument `get_jetton_data` get-method and a long transaction history.
const JETTON_MASTER: &str = "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs";

const MAX_PROVIDER_ATTEMPTS: u32 = 4;

macro_rules! live_call {
    ($provider:expr, $method:ident($($arg:expr),* $(,)?)) => {{
        let mut attempt = 0_u32;
        loop {
            request_slot().await;
            match $provider.$method($($arg),*).await {
                Ok(value) => break value,
                Err(error)
                    if attempt + 1 < MAX_PROVIDER_ATTEMPTS && is_retryable(&error) =>
                {
                    attempt += 1;
                    let delay = retry_delay(attempt);
                    eprintln!(
                        "live retry attempt={attempt} max_attempts={MAX_PROVIDER_ATTEMPTS} delay_ms={} error={error}",
                        delay.as_millis()
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }};
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn toncenter_v2_live_masterchain_info() -> Result<()> {
    let mut provider = toncenter_v2()?;
    probe_masterchain_info(&mut provider, "toncenter-v2").await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn toncenter_v2_live_account_state_and_get_method() -> Result<()> {
    let mut provider = toncenter_v2()?;
    probe_account_and_get_method(&mut provider, "toncenter-v2").await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn toncenter_v2_live_transactions() -> Result<()> {
    let mut provider = toncenter_v2()?;
    probe_transactions(&mut provider, "toncenter-v2").await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn toncenter_v3_live_masterchain_info() -> Result<()> {
    let mut provider = toncenter_v3()?;
    probe_masterchain_info(&mut provider, "toncenter-v3").await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn toncenter_v3_live_account_state_and_get_method() -> Result<()> {
    let mut provider = toncenter_v3()?;
    probe_account_and_get_method(&mut provider, "toncenter-v3").await?;
    assert_unsupported_block(provider.get_account_state(jetton_master(), Some(sample_block())))
        .await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn toncenter_v3_live_transactions() -> Result<()> {
    let mut provider = toncenter_v3()?;
    probe_transactions(&mut provider, "toncenter-v3").await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn tonapi_live_masterchain_info() -> Result<()> {
    let mut provider = tonapi()?;
    probe_masterchain_info(&mut provider, "tonapi").await
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn tonapi_live_account_state_and_get_method() -> Result<()> {
    let mut provider = tonapi()?;
    probe_account_and_get_method(&mut provider, "tonapi").await?;
    assert_unsupported_block(provider.get_account_state(jetton_master(), Some(sample_block())))
        .await?;
    let error = provider
        .run_get_method(jetton_master(), MethodRef::Id(0), TvmStack::empty(), None)
        .await
        .expect_err("TonAPI must reject numeric method ids");
    assert!(matches!(error, ProviderError::UnsupportedMethodRef { .. }));
    Ok(())
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn tonapi_live_transactions() -> Result<()> {
    let mut provider = tonapi()?;
    probe_transactions(&mut provider, "tonapi").await
}

fn toncenter_v2() -> Result<ToncenterV2> {
    Ok(ToncenterV2::builder().timeout(PROVIDER_TIMEOUT).build()?)
}

fn toncenter_v3() -> Result<ToncenterV3> {
    Ok(ToncenterV3::builder().timeout(PROVIDER_TIMEOUT).build()?)
}

fn tonapi() -> Result<TonApi> {
    Ok(TonApi::builder().timeout(PROVIDER_TIMEOUT).build()?)
}

fn jetton_master() -> Address {
    Address::parse(JETTON_MASTER).expect("the pinned USDT jetton master address must stay valid")
}

fn sample_block() -> BlockId {
    BlockId {
        workchain: -1,
        shard: i64::MIN,
        seqno: 1,
        root_hash: Int256::default(),
        file_hash: Int256::default(),
    }
}

fn is_zero_hash(hash: &Int256) -> bool {
    hash.0.iter().all(|byte| *byte == 0)
}

async fn probe_masterchain_info(
    provider: &mut impl GetMasterchainInfoExt,
    label: &str,
) -> Result<()> {
    let started = Instant::now();
    let info = live_call!(provider, get_masterchain_info());
    assert_eq!(info.last.workchain, -1, "{label}: unexpected workchain");
    assert!(info.last.seqno > 0, "{label}: empty masterchain");
    assert!(
        !is_zero_hash(&info.last.root_hash),
        "{label}: zero root hash"
    );
    assert!(
        !is_zero_hash(&info.last.file_hash),
        "{label}: zero file hash"
    );
    eprintln!(
        "live probe kind=provider provider={label} request=getMasterchainInfo elapsed_ms={} seqno={} workchain={} shard={}",
        elapsed_millis(started),
        info.last.seqno,
        info.last.workchain,
        info.last.shard,
    );
    Ok(())
}

async fn probe_account_and_get_method(provider: &mut impl Provider, label: &str) -> Result<()> {
    let started = Instant::now();
    let account = jetton_master();
    let state = live_call!(provider, get_account_state(account.clone(), None));
    assert_account_state(&state, label);

    let result = live_call!(
        provider,
        run_get_method(
            account.clone(),
            MethodRef::Name("get_jetton_data"),
            TvmStack::empty(),
            None,
        )
    );
    assert_eq!(
        result.exit_code, 0,
        "{label}: get_jetton_data failed with exit code {}",
        result.exit_code
    );
    assert!(
        matches!(result.stack.entries().first(), Some(TvmStackEntry::Int(_))),
        "{label}: get_jetton_data returned an unexpected stack"
    );
    eprintln!(
        "live probe kind=provider provider={label} request=get_account_state+runGetMethod elapsed_ms={} status={:?} balance={} code={} stack_len={}",
        elapsed_millis(started),
        state.status,
        state.balance,
        state.code.is_some(),
        result.stack.entries().len(),
    );
    Ok(())
}

async fn probe_transactions(provider: &mut impl Provider, label: &str) -> Result<()> {
    let started = Instant::now();
    let account = jetton_master();
    let state = live_call!(provider, get_account_state(account.clone(), None));
    assert_account_state(&state, label);
    let lt = state
        .last_transaction_lt
        .with_context(|| format!("{label}: account has no last transaction lt"))?;
    let hash = state
        .last_transaction_hash
        .with_context(|| format!("{label}: account has no last transaction hash"))?;

    let list = live_call!(
        provider,
        get_transactions(2_u32, account.clone(), lt, hash.clone())
    );
    assert_transactions(&list, lt, label)?;
    eprintln!(
        "live probe kind=provider provider={label} request=getTransactions elapsed_ms={} count={} first_lt={} first_hash={}",
        elapsed_millis(started),
        list.transactions.len(),
        list.transactions[0].lt,
        list.transactions[0].hash.to_hex(),
    );
    Ok(())
}

fn assert_account_state(state: &AccountState, label: &str) {
    assert_eq!(
        state.status,
        AccountStatus::Active,
        "{label}: fixture account is not active"
    );
    assert!(
        state.balance > num_bigint::BigUint::from(0_u8),
        "{label}: fixture account has no balance"
    );
    assert!(
        state.code.is_some(),
        "{label}: active account must expose code"
    );
    assert!(
        state.data.is_some(),
        "{label}: active account must expose data"
    );
}

fn assert_transactions(list: &TransactionList, lt: u64, label: &str) -> Result<()> {
    if list.transactions.is_empty() {
        bail!("{label}: account history is empty");
    }
    for transaction in &list.transactions {
        assert!(
            transaction.lt > 0,
            "{label}: transaction with zero logical time"
        );
        assert!(
            transaction.lt <= lt,
            "{label}: transaction is newer than the account state"
        );
        assert!(
            !is_zero_hash(&transaction.hash),
            "{label}: transaction with zero hash"
        );
    }
    assert!(
        list.transactions
            .windows(2)
            .all(|pair| pair[0].lt >= pair[1].lt),
        "{label}: transactions are not ordered newest first"
    );
    Ok(())
}

async fn assert_unsupported_block(
    call: impl std::future::Future<Output = ProviderResult<AccountState>>,
) -> Result<()> {
    let error = call
        .await
        .expect_err("provider must reject explicit blocks");
    assert!(matches!(error, ProviderError::UnsupportedBlock { .. }));
    Ok(())
}
