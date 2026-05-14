use super::mnemonic::*;
use super::*;
use crate::tvm::deserialize_boc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

#[test]
fn ton_mnemonic_derives_common_public_key_fixture() {
    let mnemonic = TonMnemonic::from_phrase(fixture_mnemonic(), None).unwrap();
    assert_eq!(
        hex::encode(mnemonic.public_key()),
        "0cce79e60d25fde965e249096617e3ef212541ddeb5b3336cc3853e4499da196"
    );
}

#[test]
fn ton_mnemonic_seed_derivation_matches_fixture() {
    let words = fixture_mnemonic()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let seed = ton_mnemonic_seed(&words, TON_DEFAULT_SEED, None, TON_DEFAULT_SEED_ITERATIONS);
    assert_eq!(
        hex::encode(seed),
        "0f531e609f8ccf8548fcb7f018c60a7049af809dfd112e9809b64693316af2366127fde978b0b27f4f35620b3307c31b47b488fd5fc584324f2b24f928e68eba"
    );

    let version_seed =
        ton_mnemonic_seed(&words, TON_SEED_VERSION, None, TON_SEED_VERSION_ITERATIONS);
    assert_eq!(
        hex::encode(version_seed),
        "003867fb551e82edd5f5be71a2a7c7210af5c951254afd46ad0cf282fbb0340afce22ae74d9937dda2b33ff5ffdcae7f0b4deb9dd68956b4b45a85dea010c1f2"
    );
}

#[test]
fn v5r1_generated_mnemonic_matches_upstream_default_address_fixture() {
    let mnemonic = TonMnemonic::from_phrase(
            "result state solve win angle damage shiver number art dove repeat lunch guess cement library oxygen ecology tornado era subway follow room clarify window",
            None,
        )
        .unwrap();
    assert_eq!(
        hex::encode(mnemonic.public_key()),
        "97f33453272c2d998585c19687aa9d0981c83be4c7e1fda2d35f1b058a93b8ae"
    );

    let wallet = WalletV5R1::new(
        mnemonic.public_key(),
        WALLET_V5R1_MAINNET_DEFAULT_ID,
        wallet_v5r1_code().unwrap(),
        0,
    );
    let address = wallet.address().unwrap();
    assert_eq!(
        hex::encode(address.hash_part),
        "b3806b48c4ef72119d573780ccbc4c1066d3516675158331de064cb02b117abb"
    );
    assert_eq!(
        address.to_non_bounceable(true),
        "UQCzgGtIxO9yEZ1XN4DMvEwQZtNRZnUVgzHeBkywKxF6u0Ir"
    );
}

#[test]
fn wallet_id_pack_unpack_matches_default_vectors() {
    let mainnet = WalletV5R1WalletId::mainnet_default(0);
    assert_eq!(mainnet.pack().unwrap(), WALLET_V5R1_MAINNET_DEFAULT_ID);
    assert_eq!(
        WalletV5R1WalletId::testnet_default(0).pack().unwrap(),
        WALLET_V5R1_TESTNET_DEFAULT_ID
    );
    assert_eq!(
        WalletV5R1WalletId::mainnet_default(-1).pack().unwrap(),
        0x007f_ff11
    );
    assert_eq!(
        WalletV5R1WalletId::testnet_default(-1).pack().unwrap(),
        0x007f_fffd
    );

    assert_eq!(
        WalletV5R1WalletId::unpack(WALLET_V5R1_MAINNET_DEFAULT_ID, MAINNET_GLOBAL_ID),
        mainnet
    );
}

#[test]
fn data_cell_roundtrips_with_empty_extensions() {
    let data = WalletV5R1Data::new(WALLET_V5R1_MAINNET_DEFAULT_ID, [0x11; 32]);
    let cell = data.to_cell().unwrap();
    assert_eq!(cell.bit_len(), 1 + 32 + 32 + 256 + 1);
    let decoded = WalletV5R1Data::from_cell(cell).unwrap();
    assert_eq!(decoded, data);
    assert!(decoded.extensions.is_empty());
}

#[test]
fn v5r1_extensions_insert_lookup_remove_and_roundtrip() {
    let first = [0x11; 32];
    let second = [0x22; 32];
    let first_address = Address::new(-1, first);
    let mut extensions = WalletV5R1Extensions::empty();

    assert!(extensions.is_empty());
    assert!(!extensions.insert_hash(first));
    assert!(extensions.contains_hash(first));
    assert!(extensions.contains_address(&first_address));
    assert!(extensions.insert_hash(first));
    assert_eq!(extensions.len(), 1);
    assert!(!extensions.insert_address(&Address::new(0, second)));
    assert_eq!(
        extensions.iter_hashes().collect::<Vec<_>>(),
        vec![first, second]
    );
    assert!(extensions.remove_address(&Address::new(42, first)));
    assert!(!extensions.contains_hash(first));
    assert!(!extensions.remove_hash(first));

    let cell = extensions.to_cell().unwrap();
    let decoded = WalletV5R1Extensions::from_cell(cell).unwrap();
    assert_eq!(decoded, extensions);
}

#[test]
fn v5r1_extensions_reject_invalid_key_width() {
    let mut wrong = HashmapE::new(255);
    wrong
        .insert_bit_key(BitKey::from_bits(vec![0x55; 32], 255).unwrap(), true)
        .unwrap();

    assert!(matches!(
        WalletV5R1Extensions::from_hashmap(wrong).unwrap_err(),
        WalletError::InvalidExtensionKeyWidth {
            actual: 255,
            expected: 256
        }
    ));
}

#[test]
fn v4r2_data_cell_roundtrips_with_empty_plugins() {
    let data = WalletV4R2Data::new(WALLET_V4R2_DEFAULT_ID, [0x11; 32]);
    let cell = data.to_cell().unwrap();
    assert_eq!(cell.bit_len(), 32 + 32 + 256 + 1);
    let decoded = WalletV4R2Data::from_cell(cell).unwrap();
    assert_eq!(decoded, data);
    assert!(decoded.plugins.is_empty());
}

#[test]
fn embedded_wallet_code_hashes_match_official_values() {
    let v4_first = wallet_v4r2_code().unwrap();
    let v4_second = wallet_v4r2_code().unwrap();
    assert!(Arc::ptr_eq(&v4_first, &v4_second));
    assert_eq!(v4_first.hash(), WALLET_V4R2_CODE_HASH);
    assert_eq!(v4_second.hash(), WALLET_V4R2_CODE_HASH);

    let v5_first = wallet_v5r1_code().unwrap();
    let v5_second = wallet_v5r1_code().unwrap();
    assert!(Arc::ptr_eq(&v5_first, &v5_second));
    assert_eq!(v5_first.hash(), WALLET_V5R1_CODE_HASH);
    assert_eq!(v5_second.hash(), WALLET_V5R1_CODE_HASH);
}

#[test]
fn wallet_state_init_address_fixtures_match_embedded_code() {
    let set = wallet_fixture_set();

    for fixture in set.fixtures {
        assert_fixture_metadata(&fixture);
        let public_key = hex_32(&fixture.public_key);
        let wallet_id = wallet_id_hex(&fixture.wallet_id);

        match fixture.wallet_version.as_str() {
            "V5R1" => {
                let wallet = WalletV5R1::new(
                    public_key,
                    wallet_id,
                    wallet_v5r1_code().unwrap(),
                    fixture.workchain,
                );
                assert_eq!(
                    wallet.code.hash(),
                    hex_32(&fixture.code_hash),
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    wallet.data().to_cell().unwrap().hash(),
                    hex_32(&fixture.data_hash),
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    wallet.state_init().unwrap().to_cell().unwrap().hash(),
                    hex_32(&fixture.state_init_hash),
                    "{}",
                    fixture.name
                );
                let address = wallet.address().unwrap();
                assert_eq!(address.to_raw(), fixture.raw_address, "{}", fixture.name);
                assert_eq!(
                    address.to_non_bounceable(true),
                    fixture.user_friendly_address,
                    "{}",
                    fixture.name
                );
            }
            "V4R2" => {
                let wallet = WalletV4R2::new(
                    public_key,
                    wallet_id,
                    wallet_v4r2_code().unwrap(),
                    fixture.workchain,
                );
                assert_eq!(
                    wallet.code.hash(),
                    hex_32(&fixture.code_hash),
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    wallet.data().to_cell().unwrap().hash(),
                    hex_32(&fixture.data_hash),
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    wallet.state_init().unwrap().to_cell().unwrap().hash(),
                    hex_32(&fixture.state_init_hash),
                    "{}",
                    fixture.name
                );
                let address = wallet.address().unwrap();
                assert_eq!(address.to_raw(), fixture.raw_address, "{}", fixture.name);
                assert_eq!(
                    address.to_non_bounceable(true),
                    fixture.user_friendly_address,
                    "{}",
                    fixture.name
                );
            }
            version => panic!("unexpected wallet fixture version {version}"),
        }
    }
}

#[test]
fn derived_address_is_stable_for_same_state_init() {
    let public_key = VerifyingKey::from(&signing_key()).to_bytes();
    let wallet = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let first = wallet.address().unwrap();
    let second = wallet.address().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.hash_part,
        wallet.state_init().unwrap().to_cell().unwrap().hash()
    );
}

#[test]
fn signed_external_body_verifies_against_signing_cell_hash() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key);
    let wallet = WalletV5R1::new(
        public_key.to_bytes(),
        WALLET_V5R1_MAINNET_DEFAULT_ID,
        test_code(),
        0,
    );
    let destination = Address::new(0, [0x22; 32]);
    let message = WalletMessage::internal(destination, 1_000_000).with_mode(3);

    let signing_cell = wallet
        .build_external_signing_cell(5, 1_700_000_000, vec![message.clone()])
        .unwrap();
    let signed = wallet
        .build_signed_external_body(5, 1_700_000_000, vec![message], &key)
        .unwrap();

    assert_eq!(signed.signing_hash, signing_cell.hash());
    public_key
        .verify(
            &signed.signing_hash,
            &Signature::from_bytes(&signed.signature),
        )
        .unwrap();

    let decoded = WalletV5R1ExternalBody::from_cell(signed.body).unwrap();
    assert_eq!(decoded.wallet_id, WALLET_V5R1_MAINNET_DEFAULT_ID);
    assert_eq!(decoded.valid_until, 1_700_000_000);
    assert_eq!(decoded.seqno, 5);
    assert_eq!(decoded.out_list.unwrap().len(), 1);
    assert!(decoded.extended_actions.is_none());
    assert_eq!(decoded.signature, signed.signature);
}

#[test]
fn v5r1_extended_actions_roundtrip_each_tag() {
    let address = Address::new(0, [0x33; 32]);
    let actions = vec![
        WalletV5R1ExtendedAction::add_extension(address.clone()),
        WalletV5R1ExtendedAction::delete_extension(address),
        WalletV5R1ExtendedAction::set_signature_auth_allowed(false),
    ];
    let list = WalletV5R1ExtendedActionList::new(actions.clone()).unwrap();
    let cell = list.to_cell().unwrap();
    assert_eq!(cell.references().len(), 1);

    let decoded = WalletV5R1ExtendedActionList::from_cell(cell).unwrap();
    assert_eq!(decoded.actions, actions);
}

#[test]
fn v5r1_signed_body_decodes_mixed_ordinary_and_extended_actions() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key);
    let wallet = WalletV5R1::new(
        public_key.to_bytes(),
        WALLET_V5R1_MAINNET_DEFAULT_ID,
        test_code(),
        0,
    );
    let destination = Address::new(0, [0x44; 32]);
    let message = WalletMessage::internal(destination.clone(), 10);
    let extended = vec![
        WalletV5R1ExtendedAction::add_extension(destination),
        WalletV5R1ExtendedAction::set_signature_auth_allowed(true),
    ];

    let signed = wallet
        .build_signed_external_body_with_extended_actions(
            9,
            1_700_000_009,
            vec![message],
            extended.clone(),
            &key,
        )
        .unwrap();
    public_key
        .verify(
            &signed.signing_hash,
            &Signature::from_bytes(&signed.signature),
        )
        .unwrap();

    let decoded = WalletV5R1ExternalBody::from_cell(signed.body).unwrap();
    assert_eq!(decoded.out_list.unwrap().len(), 1);
    assert_eq!(decoded.extended_actions.unwrap().actions, extended);
}

#[test]
fn v4r2_signed_external_body_verifies_against_signing_cell_hash() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key);
    let wallet = WalletV4R2::new(
        public_key.to_bytes(),
        WALLET_V4R2_DEFAULT_ID,
        test_code(),
        0,
    );
    let destination = Address::new(0, [0x22; 32]);
    let message = WalletMessage::internal(destination, 1_000_000).with_mode(3);

    let signing_cell = wallet
        .build_external_signing_cell(5, 1_700_000_000, vec![message.clone()])
        .unwrap();
    let signed = wallet
        .build_signed_external_body(5, 1_700_000_000, vec![message], &key)
        .unwrap();

    assert_eq!(signed.signing_hash, signing_cell.hash());
    public_key
        .verify(
            &signed.signing_hash,
            &Signature::from_bytes(&signed.signature),
        )
        .unwrap();

    let mut slice = Slice::new(signed.body);
    assert_eq!(slice.load_bytes(64).unwrap(), signed.signature);
    assert_eq!(slice.load_u32().unwrap(), WALLET_V4R2_DEFAULT_ID);
    assert_eq!(slice.load_u32().unwrap(), 1_700_000_000);
    assert_eq!(slice.load_u32().unwrap(), 5);
    assert_eq!(slice.load_u32().unwrap(), 0);
}

#[test]
fn rejects_more_than_255_wallet_messages() {
    let public_key = VerifyingKey::from(&signing_key()).to_bytes();
    let wallet = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let messages = vec![WalletMessage::internal(Address::new(0, [1; 32]), 1); 256];
    let err = wallet
        .build_external_signing_cell(0, 1, messages)
        .unwrap_err();
    assert!(matches!(
        err,
        WalletError::TooManyActions {
            count: 256,
            max: 255
        }
    ));
}

#[test]
fn v5r1_rejects_more_than_255_total_actions() {
    let public_key = VerifyingKey::from(&signing_key()).to_bytes();
    let wallet = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let messages = vec![WalletMessage::internal(Address::new(0, [1; 32]), 1); 255];
    let extended = vec![WalletV5R1ExtendedAction::set_signature_auth_allowed(true)];
    let err = wallet
        .build_external_signing_cell_with_extended_actions(0, 1, messages, extended)
        .unwrap_err();
    assert!(matches!(
        err,
        WalletError::TooManyActions {
            count: 256,
            max: 255
        }
    ));
}

#[test]
fn v4r2_rejects_more_than_4_wallet_messages() {
    let public_key = VerifyingKey::from(&signing_key()).to_bytes();
    let wallet = WalletV4R2::new(public_key, WALLET_V4R2_DEFAULT_ID, test_code(), 0);
    let messages = vec![WalletMessage::internal(Address::new(0, [1; 32]), 1); 5];
    let err = wallet
        .build_external_signing_cell(0, 1, messages)
        .unwrap_err();
    assert!(matches!(
        err,
        WalletError::TooManyActions { count: 5, max: 4 }
    ));
}

#[test]
fn external_message_boc_decodes_as_message() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key).to_bytes();
    let wallet = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let boc = wallet
        .build_external_message_boc(0, 1_700_000_001, Vec::new(), &key, true)
        .unwrap();
    let decoded = Message::from_cell(deserialize_boc(&boc).unwrap()).unwrap();
    match decoded.info {
        CommonMsgInfo::ExternalIn { dest, .. } => {
            assert_eq!(dest, MsgAddressInt::std(wallet.address().unwrap()));
        }
        _ => panic!("expected external inbound message"),
    }
    assert!(decoded.init.is_some());
}

#[test]
fn v4r2_external_message_boc_decodes_as_message() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key).to_bytes();
    let wallet = WalletV4R2::new(public_key, WALLET_V4R2_DEFAULT_ID, test_code(), 0);
    let boc = wallet
        .build_external_message_boc(0, 1_700_000_001, Vec::new(), &key, true)
        .unwrap();
    let decoded = Message::from_cell(deserialize_boc(&boc).unwrap()).unwrap();
    match decoded.info {
        CommonMsgInfo::ExternalIn { dest, .. } => {
            assert_eq!(dest, MsgAddressInt::std(wallet.address().unwrap()));
        }
        _ => panic!("expected external inbound message"),
    }
    assert!(decoded.init.is_some());
}
