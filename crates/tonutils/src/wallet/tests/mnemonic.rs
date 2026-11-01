use ed25519_dalek::SigningKey;
use serde::Deserialize;
use std::sync::Arc;
use tonutils::tvm::Cell;

use crate::wallet::wallet_v5r1_code;

#[derive(Debug, Deserialize)]
pub(super) struct WalletFixtureSet {
    pub(super) schema_revision: String,
    pub(super) fixtures: Vec<WalletFixture>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WalletFixture {
    pub(super) name: String,
    pub(super) source: String,
    pub(super) capture_date: String,
    pub(super) upstream_reference: String,
    pub(super) wallet_version: String,
    pub(super) network: String,
    pub(super) public_key: String,
    pub(super) workchain: i8,
    pub(super) wallet_id: String,
    pub(super) code_hash: String,
    pub(super) data_hash: String,
    pub(super) state_init_hash: String,
    pub(super) raw_address: String,
    pub(super) user_friendly_address: String,
}

pub(super) fn wallet_fixture_set() -> WalletFixtureSet {
    let set: WalletFixtureSet = serde_json::from_str(include_str!(
        "../../../../../fixtures/wallets/state_init_addresses.json"
    ))
    .expect("wallet state-init fixture must be valid JSON");
    assert_ne!(set.schema_revision, "");
    assert!(!set.fixtures.is_empty());
    set
}

pub(super) fn assert_fixture_metadata(fixture: &WalletFixture) {
    assert_ne!(fixture.name, "");
    assert_ne!(fixture.source, "");
    assert_ne!(fixture.capture_date, "");
    assert!(fixture.upstream_reference.starts_with("https://"));
    assert!(matches!(fixture.wallet_version.as_str(), "V4R2" | "V5R1"));
    assert_ne!(fixture.network, "");
}

pub(super) fn wallet_id_hex(value: &str) -> u32 {
    let value = value
        .strip_prefix("0x")
        .expect("fixture wallet ID must use a hexadecimal prefix");
    u32::from_str_radix(value, 16).expect("fixture wallet ID must be valid hexadecimal")
}

pub(super) fn fixture_mnemonic() -> &'static str {
    "result state solve win angle damage shiver number art dove repeat lunch guess cement library oxygen ecology tornado era subway follow room clarify window"
}

pub(super) fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

pub(super) fn test_code() -> Arc<Cell> {
    wallet_v5r1_code().expect("embedded wallet code is valid")
}

pub(super) fn hex_32(value: &str) -> [u8; 32] {
    hex::decode(value)
        .expect("fixture is valid hex")
        .try_into()
        .expect("fixture contains 32 bytes")
}
