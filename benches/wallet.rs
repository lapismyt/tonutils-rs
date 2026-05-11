use criterion::{Criterion, criterion_group, criterion_main};
use ed25519_dalek::VerifyingKey;
use rand::{SeedableRng, rngs::StdRng};
use std::time::Duration;
use tonutils::tvm::{Address, Builder};
use tonutils::wallet::{
    MAINNET_GLOBAL_ID, TonMnemonic, WALLET_V4R2_DEFAULT_ID, WALLET_V5R1_MAINNET_DEFAULT_ID,
    WalletMessage, WalletV4R2, WalletV5R1, WalletV5R1WalletId, ton_mnemonic_seed, wallet_v4r2_code,
    wallet_v5r1_code,
};

const FIXTURE_MNEMONIC: &str = "open price dish charge law skirt alien churn fire swap number brass outdoor diamond lesson april remain puzzle title elbow valley grant champion staff";
const TON_DEFAULT_SEED: &[u8] = b"TON default seed";
const TON_SEED_VERSION: &[u8] = b"TON seed version";
const TON_DEFAULT_SEED_ITERATIONS: usize = 100_000;
const TON_SEED_VERSION_ITERATIONS: usize = 100_000 / 256;

fn fixture_words() -> Vec<String> {
    FIXTURE_MNEMONIC
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

fn fixture_wallets() -> (TonMnemonic, WalletV4R2, WalletV5R1, WalletMessage) {
    let mnemonic = TonMnemonic::from_phrase(FIXTURE_MNEMONIC, None).unwrap();
    let public_key = mnemonic.public_key();
    let v4 = WalletV4R2::new(
        public_key,
        WALLET_V4R2_DEFAULT_ID,
        wallet_v4r2_code().unwrap(),
        0,
    );
    let v5 = WalletV5R1::new(
        public_key,
        WALLET_V5R1_MAINNET_DEFAULT_ID,
        wallet_v5r1_code().unwrap(),
        0,
    );
    let mut body = Builder::new();
    body.store_u32(0).unwrap();
    body.store_bytes(b"criterion wallet benchmark").unwrap();
    let message = WalletMessage::internal(Address::new(0, [0x22; 32]), 1_000_000_000)
        .with_body(body.build().unwrap());
    (mnemonic, v4, v5, message)
}

fn bench_mnemonics(c: &mut Criterion) {
    let mut group = c.benchmark_group("wallet_mnemonic");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));

    group.bench_function("mnemonic_import_derive_key", |b| {
        b.iter(|| TonMnemonic::from_phrase(FIXTURE_MNEMONIC, None).unwrap())
    });

    let words = fixture_words();
    group.bench_function("mnemonic_seed_default_100k", |b| {
        b.iter(|| ton_mnemonic_seed(&words, TON_DEFAULT_SEED, None, TON_DEFAULT_SEED_ITERATIONS))
    });

    group.bench_function("mnemonic_seed_version_check", |b| {
        b.iter(|| ton_mnemonic_seed(&words, TON_SEED_VERSION, None, TON_SEED_VERSION_ITERATIONS))
    });

    group.bench_function("mnemonic_generate_deterministic_rng", |b| {
        let mut rng = StdRng::seed_from_u64(0x746f_6e75_7469_6c73);
        b.iter(|| TonMnemonic::generate_with_rng(None, &mut rng).unwrap())
    });

    group.finish();
}

fn bench_wallets(c: &mut Criterion) {
    let mut group = c.benchmark_group("wallet");
    group.sample_size(20);

    group.bench_function("wallet_code_v4r2_cached", |b| {
        b.iter(|| wallet_v4r2_code().unwrap())
    });
    group.bench_function("wallet_code_v5r1_cached", |b| {
        b.iter(|| wallet_v5r1_code().unwrap())
    });

    let (mnemonic, v4, v5, message) = fixture_wallets();
    let signing_key = mnemonic.signing_key();
    let verifying_key = VerifyingKey::from(signing_key);
    assert_eq!(verifying_key.to_bytes(), mnemonic.public_key());
    assert_eq!(
        WalletV5R1WalletId::client(MAINNET_GLOBAL_ID, 0, 0, 0)
            .pack()
            .unwrap(),
        WALLET_V5R1_MAINNET_DEFAULT_ID
    );

    group.bench_function("wallet_v4r2_address", |b| b.iter(|| v4.address().unwrap()));
    group.bench_function("wallet_v5r1_address", |b| b.iter(|| v5.address().unwrap()));
    group.bench_function("wallet_v4r2_signed_transfer_boc", |b| {
        b.iter(|| {
            v4.build_external_message_boc(
                7,
                1_700_000_000,
                vec![message.clone()],
                signing_key,
                true,
            )
            .unwrap()
        })
    });
    group.bench_function("wallet_v5r1_signed_transfer_boc", |b| {
        b.iter(|| {
            v5.build_external_message_boc(
                7,
                1_700_000_000,
                vec![message.clone()],
                signing_key,
                true,
            )
            .unwrap()
        })
    });
    group.bench_function("wallet_message_comment_body", |b| {
        b.iter(|| {
            let mut body = Builder::new();
            body.store_u32(0).unwrap();
            body.store_bytes(b"criterion wallet benchmark").unwrap();
            body.build().unwrap()
        })
    });

    group.finish();
}

criterion_group!(benches, bench_mnemonics, bench_wallets);
criterion_main!(benches);
