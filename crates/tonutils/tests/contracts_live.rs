#![cfg(all(feature = "provider", feature = "jetton", feature = "nft"))]

//! Live mainnet probes for the jetton and NFT contract wrappers.
//!
//! The probes read a pinned master/collection from mainnet, decode its stored
//! data with the wrapper data models, and compare on-chain get-methods with
//! offline address derivation. They never submit messages.

mod provider_live_common;

use std::time::Instant;

use anyhow::{Context, Result, bail};
use provider_live_common::{
    PROVIDER_TIMEOUT, elapsed_millis, is_retryable, request_slot, retry_delay,
};
use tonutils::{
    contracts::ContractError,
    jetton::{
        JettonMasterStandard, JettonMasterStandardData, get_jetton_data, get_wallet_address,
        get_wallet_data,
    },
    nft::{
        NFTCollectionData, calculate_item_address, get_collection_data, get_nft_address_by_index,
        get_nft_data,
    },
    provider::{GetAccountStateExt, ToncenterV3},
    tlb::{AccountStatus, TlbDeserialize},
    tvm::{Address, Slice},
};

/// A TEP-74 standard jetton master on mainnet using the embedded standard
/// master code hash.
const JETTON_MASTER: &str = "EQAAOgn7wyNCV4Yw7c238Plo4xeHhP/9ekjB6f9OxPVKfQfX";

/// A TEP-62 standard collection with minted items.
const NFT_COLLECTION: &str = "EQAAORTCHU+tVFvCQn/pakqX8bv5mmd+SloLPzToKHSlm+lw";

const MAX_PROVIDER_ATTEMPTS: u32 = 4;

macro_rules! live_provider_call {
    ($call:expr) => {{
        let mut attempt = 0_u32;
        loop {
            request_slot().await;
            match $call.await {
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

macro_rules! live_contract_call {
    ($call:expr) => {{
        let mut attempt = 0_u32;
        loop {
            request_slot().await;
            match $call.await {
                Ok(value) => break value,
                Err(ContractError::Provider(error))
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
async fn live_jetton_standard_master_and_wallet() -> Result<()> {
    let started = Instant::now();
    let mut provider = toncenter_v3()?;
    let master = Address::parse(JETTON_MASTER).expect("pinned jetton master address must parse");

    let state = live_provider_call!(provider.get_account_state(master.clone(), None));
    let data = state
        .data
        .clone()
        .with_context(|| format!("{JETTON_MASTER}: master account has no data"))?;
    let stored = JettonMasterStandardData::load_tlb(&mut Slice::new(data))
        .context("standard jetton master data must decode")?;

    let info = live_contract_call!(get_jetton_data(&mut provider, master.clone()));
    assert_eq!(
        info.total_supply,
        num_bigint::BigUint::from(stored.total_supply),
        "get_jetton_data supply must match stored data"
    );
    assert_eq!(
        info.jetton_wallet_code.hash(),
        stored.jetton_wallet_code.hash(),
        "get_jetton_data wallet code must match stored data"
    );

    let owner = Address::parse("EQD9Sl0m2UU5mWgO6F5aNg2DJzHlcTP+PYmwitBrJ87J6+Ep")
        .expect("owner address must parse");
    let on_chain = live_contract_call!(get_wallet_address(&mut provider, master.clone(), &owner,));
    let offline = JettonMasterStandard::calculate_wallet_address(
        &owner,
        &master,
        &info.jetton_wallet_code,
        master.workchain,
    )
    .context("offline jetton wallet derivation must succeed")?;
    assert_eq!(
        on_chain.to_raw(),
        offline.to_raw(),
        "offline jetton wallet address must match the master"
    );

    let wallet_state = live_provider_call!(provider.get_account_state(on_chain.clone(), None));
    if wallet_state.status == AccountStatus::Active {
        let wallet = live_contract_call!(get_wallet_data(&mut provider, on_chain.clone()));
        assert_eq!(
            wallet.owner_address.to_raw(),
            owner.to_raw(),
            "live jetton wallet owner must match the queried owner"
        );
        assert_eq!(
            wallet.jetton_master_address.to_raw(),
            master.to_raw(),
            "live jetton wallet master must match the queried master"
        );
    }

    eprintln!(
        "live probe kind=contract provider=toncenter-v3 request=jetton elapsed_ms={} supply={} wallet_code_matches=true",
        elapsed_millis(started),
        info.total_supply,
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live mainnet HTTP access"]
async fn live_nft_standard_collection_and_item() -> Result<()> {
    let started = Instant::now();
    let mut provider = toncenter_v3()?;
    let collection =
        Address::parse(NFT_COLLECTION).expect("pinned NFT collection address must parse");

    let state = live_provider_call!(provider.get_account_state(collection.clone(), None));
    let data = state
        .data
        .clone()
        .with_context(|| format!("{NFT_COLLECTION}: collection account has no data"))?;
    let stored = NFTCollectionData::load_tlb(&mut Slice::new(data))
        .context("standard NFT collection data must decode")?;

    let info = live_contract_call!(get_collection_data(&mut provider, collection.clone()));
    assert_eq!(
        info.next_item_index, stored.next_item_index,
        "get_collection_data index must match stored data"
    );

    if stored.next_item_index == 0 {
        bail!("pinned NFT collection has no minted items");
    }
    let index = 0_u64;
    let on_chain = live_contract_call!(get_nft_address_by_index(
        &mut provider,
        collection.clone(),
        index
    ));
    let offline = calculate_item_address(
        index,
        &collection,
        &stored.nft_item_code,
        collection.workchain,
    )
    .context("offline NFT item derivation must succeed")?;
    assert_eq!(
        on_chain.to_raw(),
        offline.to_raw(),
        "offline NFT item address must match the collection"
    );

    let item = live_contract_call!(get_nft_data(&mut provider, on_chain.clone()));
    assert!(item.initialized, "live NFT item must be initialized");
    assert_eq!(item.index, index, "live NFT item index must match");
    assert_eq!(
        item.collection_address.as_ref().map(Address::to_raw),
        Some(collection.to_raw()),
        "live NFT item collection must match the queried collection"
    );

    eprintln!(
        "live probe kind=contract provider=toncenter-v3 request=nft elapsed_ms={} next_item_index={}",
        elapsed_millis(started),
        info.next_item_index,
    );
    Ok(())
}

fn toncenter_v3() -> Result<ToncenterV3> {
    Ok(ToncenterV3::builder().timeout(PROVIDER_TIMEOUT).build()?)
}
