//! Offline wallet helpers.
//!
//! The first wallet surface covers offline Wallet V4R2 and V5R1 helpers.
//! It intentionally starts with deterministic cell construction, address
//! derivation, signing, and external message BoC assembly; live send helpers
//! are thin provider adapters.

use crate::tlb::{
    CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Either, Grams, Message,
    MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, OutAction, OutList, StateInit,
    TlbDeserialize, TlbError, TlbSerialize, ensure_empty,
};
use crate::tvm::{Address, Builder, Cell, HashmapE, Slice, serialize_boc};
use bip39::{Language, Mnemonic};
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use num_bigint::BigUint;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha512;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;

const TON_MNEMONIC_WORD_COUNT: usize = 24;
const TON_DEFAULT_SEED: &[u8] = b"TON default seed";
const TON_SEED_VERSION: &[u8] = b"TON seed version";
const TON_FAST_SEED_VERSION: &[u8] = b"TON fast seed version";
const TON_DEFAULT_SEED_ITERATIONS: usize = 100_000;
const TON_SEED_VERSION_ITERATIONS: usize = 100_000 / 256;
const TON_FAST_SEED_VERSION_ITERATIONS: usize = 1;

const WALLET_V4R2_PLUGINS_KEY_BITS: usize = 256;
const WALLET_V4R2_MAX_MESSAGES: usize = 4;
const WALLET_V4R2_SIMPLE_SEND_OP: u32 = 0;
/// Default Wallet V4R2 wallet id used by common TON wallets.
pub const WALLET_V4R2_DEFAULT_ID: u32 = 0x29a9_a317;
const WALLET_V4R2_CODE_BOC_HEX: &str = "b5ee9c72010214010002d4000114ff00f4a413f4bcf2c80b01020120020f020148030602e6d001d0d3032171b0925f04e022d749c120925f04e002d31f218210706c7567bd22821064737472bdb0925f05e003fa403020fa4401c8ca07cbffc9d0ed44d0810140d721f404305c810108f40a6fa131b3925f07e005d33fc8258210706c7567ba923830e30d03821064737472ba925f06e30d0405007801fa00f40430f8276f2230500aa121bef2e0508210706c7567831eb17080185004cb0526cf1658fa0219f400cb6917cb1f5260cb3f20c98040fb0006008a5004810108f45930ed44d0810140d720c801cf16f400c9ed540172b08e23821064737472831eb17080185005cb055003cf1623fa0213cb6acb1fcb3fc98040fb00925f03e2020120070e020120080d020158090a003db29dfb513420405035c87d010c00b23281f2fff274006040423d029be84c600201200b0c0019adce76a26840206b90eb85ffc00019af1df6a26840106b90eb858fc00011b8c97ed44d0d70b1f80059bd242b6f6a2684080a06b90fa0218470d4080847a4937d29910ce6903e9ff9837812801b7810148987159f318404f8f28308d71820d31fd31fd31f02f823bbf264ed44d0d31fd31fd3fff404d15143baf2a15151baf2a205f901541064f910f2a3f80024a4c8cb1f5240cb1f5230cbff5210f400c9ed54f80f01d30721c0009f6c519320d74a96d307d402fb00e830e021c001e30021c002e30001c0039130e30d03a4c8cb1f12cb1fcbff10111213006ed207fa00d4d422f90005c8ca0715cbffc9d077748018c8cb05cb0222cf165005fa0214cb6b12ccccc973fb00c84014810108f451f2a7020070810108d718fa00d33fc8542047810108f451f2a782106e6f746570748018c8cb05cb025006cf165004fa0214cb6a12cb1fcb3fc973fb0002006c810108d718fa00d33f305224810108f459f2a782106473747270748018c8cb05cb025005cf165003fa0213cb6acb1f12cb3fc973fb00000af400c9ed54";

const WALLET_V5R1_EXTERNAL_SIGNED_OP: u32 = 0x7369_676e;
const WALLET_V5R1_EXTENSIONS_KEY_BITS: usize = 256;
const WALLET_V5R1_MAX_ACTIONS: usize = 255;
const WALLET_V5R1_CLIENT_CONTEXT_FLAG: u32 = 1 << 31;
const WALLET_V5R1_CLIENT_SUBWALLET_BITS: u32 = 15;
const WALLET_V5R1_CLIENT_SUBWALLET_MAX: u16 = (1 << WALLET_V5R1_CLIENT_SUBWALLET_BITS) - 1;
const WALLET_V5R1_CODE_BOC_HEX: &str = "b5ee9c7241021401000281000114ff00f4a413f4bcf2c80b01020120020d020148030402dcd020d749c120915b8f6320d70b1f2082106578746ebd21821073696e74bdb0925f03e082106578746eba8eb48020d72101d074d721fa4030fa44f828fa443058bd915be0ed44d0810141d721f4058307f40e6fa1319130e18040d721707fdb3ce03120d749810280b99130e070e2100f020120050c020120060902016e07080019adce76a2684020eb90eb85ffc00019af1df6a2684010eb90eb858fc00201480a0b0017b325fb51341c75c875c2c7e00011b262fb513435c280200019be5f0f6a2684080a0eb90fa02c0102f20e011e20d70b1f82107369676ebaf2e08a7f0f01e68ef0eda2edfb218308d722028308d723208020d721d31fd31fd31fed44d0d200d31f20d31fd3ffd70a000af90140ccf9109a28945f0adb31e1f2c087df02b35007b0f2d0845125baf2e0855036baf2e086f823bbf2d0882292f800de01a47fc8ca00cb1f01cf16c9ed542092f80fde70db3cd81003f6eda2edfb02f404216e926c218e4c0221d73930709421c700b38e2d01d72820761e436c20d749c008f2e09320d74ac002f2e09320d71d06c712c2005230b0f2d089d74cd7393001a4e86c128407bbf2e093d74ac000f2e093ed55e2d20001c000915be0ebd72c08142091709601d72c081c12e25210b1e30f20d74a111213009601fa4001fa44f828fa443058baf2e091ed44d0810141d718f405049d7fc8ca0040048307f453f2e08b8e14038307f45bf2e08c22d70a00216e01b3b0f2d090e2c85003cf1612f400c9ed54007230d72c08248e2d21f2e092d200ed44d0d2005113baf2d08f54503091319c01810140d721d70a00f2e08ee2c8ca0058cf16c9ed5493f2c08de20010935bdb31e1d74cd0b4d6c35e";

static WALLET_V4R2_CODE: OnceLock<Result<Arc<Cell>, String>> = OnceLock::new();
static WALLET_V5R1_CODE: OnceLock<Result<Arc<Cell>, String>> = OnceLock::new();

/// Wallet V4R2 code hash for the embedded `@ton/ton` V4R2 code BoC.
pub const WALLET_V4R2_CODE_HASH: [u8; 32] = [
    0xfe, 0xb5, 0xff, 0x68, 0x20, 0xe2, 0xff, 0x0d, 0x94, 0x83, 0xe7, 0xe0, 0xd6, 0x2c, 0x81, 0x7d,
    0x84, 0x67, 0x89, 0xfb, 0x4a, 0xe5, 0x80, 0xc8, 0x78, 0x86, 0x6d, 0x95, 0x9d, 0xab, 0xd5, 0xc0,
];

/// Wallet V5R1 code hash for the embedded `@ton/ton` V5R1 code BoC.
pub const WALLET_V5R1_CODE_HASH: [u8; 32] = [
    0x20, 0x83, 0x4b, 0x7b, 0x72, 0xb1, 0x12, 0x14, 0x7e, 0x1b, 0x2f, 0xb4, 0x57, 0xb8, 0x4e, 0x74,
    0xd1, 0xa3, 0x0f, 0x04, 0xf7, 0x37, 0xd4, 0xf6, 0x2a, 0x66, 0x8e, 0x95, 0x52, 0xd2, 0xb7, 0x2f,
];

/// Mainnet global id used by the V5 wallet id derivation.
pub const MAINNET_GLOBAL_ID: i32 = -239;

/// Testnet global id used by the V5 wallet id derivation.
pub const TESTNET_GLOBAL_ID: i32 = -3;

/// Default V5R1 wallet id for mainnet, workchain 0, wallet version 0, subwallet 0.
pub const WALLET_V5R1_MAINNET_DEFAULT_ID: u32 = 0x7fff_ff11;

/// Default V5R1 wallet id for testnet, workchain 0, wallet version 0, subwallet 0.
pub const WALLET_V5R1_TESTNET_DEFAULT_ID: u32 = 0x7fff_fffd;

/// Errors returned by Wallet V5R1 helper construction.
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("mnemonic must contain 24 words, got {0}")]
    InvalidMnemonicWordCount(usize),
    #[error("mnemonic contains a word outside the BIP-39 English word list: {0}")]
    UnknownMnemonicWord(String),
    #[error("mnemonic does not match TON seed version checks")]
    InvalidMnemonicSeedVersion,
    #[error("failed to decode embedded wallet code: {0}")]
    CodeDecode(String),
    #[error("Wallet V5R1 client subwallet number {0} exceeds 15-bit maximum 32767")]
    SubwalletNumberTooLarge(u32),
    #[error("Wallet V5R1 custom context {0} exceeds 31-bit maximum")]
    CustomContextTooLarge(u32),
    #[error("Wallet V5R1 action count {count} exceeds maximum {max}")]
    TooManyActions { count: usize, max: usize },
    #[error("failed to serialize wallet TL-B value: {0}")]
    Tlb(#[from] TlbError),
    #[error("failed to build wallet cell or BoC: {0}")]
    Tvm(#[from] anyhow::Error),
}

/// Wallet contract versions supported by the offline helpers and CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletVersion {
    /// Wallet V4R2.
    V4R2,
    /// Wallet V5R1.
    V5R1,
}

/// TON mnemonic material and derived Ed25519 signing key.
#[derive(Debug, Clone)]
pub struct TonMnemonic {
    words: Vec<String>,
    signing_key: SigningKey,
}

impl TonMnemonic {
    /// Generates a 24-word TON mnemonic using the English BIP-39 word list and
    /// TON seed-version checks.
    pub fn generate(password: Option<&str>) -> Result<Self, WalletError> {
        let worker_count = thread::available_parallelism()
            .map(|count| count.get().min(8))
            .unwrap_or(1);
        if worker_count > 1 {
            return Self::generate_with_parallel_os_rng(password, worker_count);
        }
        let mut rng = rand::rngs::OsRng;
        Self::generate_with_rng(password, &mut rng)
    }

    /// Generates a 24-word TON mnemonic using the supplied random source.
    ///
    /// This is primarily useful for deterministic tests and benchmarks. Normal
    /// wallet creation should use [`TonMnemonic::generate`].
    pub fn generate_with_rng<R>(password: Option<&str>, rng: &mut R) -> Result<Self, WalletError>
    where
        R: RngCore + ?Sized,
    {
        loop {
            let mut entropy = [0u8; 32];
            rng.fill_bytes(&mut entropy);
            let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
                .map_err(|error| WalletError::CodeDecode(error.to_string()))?;
            let phrase = mnemonic.to_string();
            if validate_ton_mnemonic_phrase(&phrase, password).is_ok() {
                let words = mnemonic.words().map(str::to_owned).collect::<Vec<_>>();
                return Self::from_validated_words(words, password);
            }
        }
    }

    fn generate_with_parallel_os_rng(
        password: Option<&str>,
        worker_count: usize,
    ) -> Result<Self, WalletError> {
        let found = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();

        let words = thread::scope(|scope| {
            for _ in 0..worker_count {
                let tx = tx.clone();
                let found = &found;
                scope.spawn(move || {
                    let mut rng = rand::rngs::OsRng;
                    while !found.load(Ordering::Acquire) {
                        let mut entropy = [0u8; 32];
                        rng.fill_bytes(&mut entropy);
                        let mnemonic = match Mnemonic::from_entropy_in(Language::English, &entropy)
                        {
                            Ok(mnemonic) => mnemonic,
                            Err(error) => {
                                if found
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Acquire,
                                    )
                                    .is_ok()
                                {
                                    let _ =
                                        tx.send(Err(WalletError::CodeDecode(error.to_string())));
                                }
                                break;
                            }
                        };
                        let phrase = mnemonic.to_string();
                        if validate_ton_mnemonic_phrase(&phrase, password).is_ok()
                            && found
                                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                        {
                            let words = mnemonic.words().map(str::to_owned).collect::<Vec<_>>();
                            let _ = tx.send(Ok(words));
                            break;
                        }
                    }
                });
            }
            drop(tx);
            rx.recv()
                .unwrap_or_else(|_| Err(WalletError::InvalidMnemonicSeedVersion))
        })?;

        Self::from_validated_words(words, password)
    }

    /// Imports and validates a TON mnemonic.
    pub fn from_phrase(phrase: &str, password: Option<&str>) -> Result<Self, WalletError> {
        let words = phrase
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Self::from_words(words, password)
    }

    /// Imports and validates TON mnemonic words.
    pub fn from_words(words: Vec<String>, password: Option<&str>) -> Result<Self, WalletError> {
        validate_ton_mnemonic_words(&words, password)?;
        Self::from_validated_words(words, password)
    }

    fn from_validated_words(
        words: Vec<String>,
        password: Option<&str>,
    ) -> Result<Self, WalletError> {
        let seed = ton_mnemonic_seed(
            &words,
            TON_DEFAULT_SEED,
            password,
            TON_DEFAULT_SEED_ITERATIONS,
        );
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&seed[..32]);
        Ok(Self {
            words,
            signing_key: SigningKey::from_bytes(&secret),
        })
    }

    /// Returns mnemonic words.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Returns the mnemonic phrase joined by spaces.
    pub fn phrase(&self) -> String {
        self.words.join(" ")
    }

    /// Returns the Ed25519 signing key derived from the TON default seed.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// Returns the Ed25519 public key bytes.
    pub fn public_key(&self) -> [u8; 32] {
        ed25519_dalek::VerifyingKey::from(&self.signing_key).to_bytes()
    }
}

/// Derives the 64-byte TON seed for a mnemonic and seed domain.
pub fn ton_mnemonic_seed(
    words: &[String],
    seed: &[u8],
    password: Option<&str>,
    iterations: usize,
) -> [u8; 64] {
    let phrase = words.join(" ");
    ton_mnemonic_seed_from_phrase(&phrase, seed, password, iterations)
}

fn ton_mnemonic_seed_from_phrase(
    phrase: &str,
    seed: &[u8],
    password: Option<&str>,
    iterations: usize,
) -> [u8; 64] {
    let mut mac = Hmac::<Sha512>::new_from_slice(phrase.as_bytes())
        .expect("HMAC-SHA512 accepts keys of any length");
    mac.update(password.unwrap_or("").as_bytes());
    let hmac = mac.finalize().into_bytes();
    let mut out = [0u8; 64];
    pbkdf2_hmac::<Sha512>(
        &hmac,
        seed,
        iterations
            .try_into()
            .expect("TON mnemonic PBKDF2 iteration count fits in u32"),
        &mut out,
    );
    out
}

fn validate_ton_mnemonic_phrase(phrase: &str, password: Option<&str>) -> Result<(), WalletError> {
    let seed = if password.is_some() {
        ton_mnemonic_seed_from_phrase(
            phrase,
            TON_FAST_SEED_VERSION,
            password,
            TON_FAST_SEED_VERSION_ITERATIONS,
        )
    } else {
        ton_mnemonic_seed_from_phrase(phrase, TON_SEED_VERSION, None, TON_SEED_VERSION_ITERATIONS)
    };
    validate_ton_seed_version_byte(seed[0], password)
}

fn validate_ton_mnemonic_words(
    words: &[String],
    password: Option<&str>,
) -> Result<(), WalletError> {
    if words.len() != TON_MNEMONIC_WORD_COUNT {
        return Err(WalletError::InvalidMnemonicWordCount(words.len()));
    }
    let wordlist = Language::English.word_list();
    for word in words {
        if wordlist.binary_search(&word.as_str()).is_err() {
            return Err(WalletError::UnknownMnemonicWord(word.clone()));
        }
    }
    let seed = if password.is_some() {
        ton_mnemonic_seed(
            words,
            TON_FAST_SEED_VERSION,
            password,
            TON_FAST_SEED_VERSION_ITERATIONS,
        )
    } else {
        ton_mnemonic_seed(words, TON_SEED_VERSION, None, TON_SEED_VERSION_ITERATIONS)
    };
    validate_ton_seed_version_byte(seed[0], password)
}

fn validate_ton_seed_version_byte(byte: u8, password: Option<&str>) -> Result<(), WalletError> {
    let expected = if password.is_some() { 1 } else { 0 };
    if byte != expected {
        return Err(WalletError::InvalidMnemonicSeedVersion);
    }
    Ok(())
}

/// Errors returned by live wallet send helpers.
#[cfg(feature = "liteclient")]
#[derive(Debug, thiserror::Error)]
pub enum WalletSendError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("failed to build wallet external message: {0}")]
    Build(#[from] WalletError),
    #[error("contract provider error: {0}")]
    Provider(#[source] E),
}

/// Persistent Wallet V5R1 storage data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV5R1Data {
    /// Whether signature authentication through `public_key` is allowed.
    pub is_signature_allowed: bool,
    /// Current wallet sequence number.
    pub seqno: u32,
    /// V5 wallet id, stored as raw 32 bits.
    pub wallet_id: u32,
    /// Ed25519 public key bytes.
    pub public_key: [u8; 32],
    /// Extension dictionary keyed by 256-bit address hash, with `int1` values.
    pub extensions: HashmapE<bool>,
}

impl WalletV5R1Data {
    /// Creates initial Wallet V5R1 data with signatures enabled, seqno zero,
    /// and an empty extension dictionary.
    pub fn new(wallet_id: u32, public_key: [u8; 32]) -> Self {
        Self {
            is_signature_allowed: true,
            seqno: 0,
            wallet_id,
            public_key,
            extensions: HashmapE::new(WALLET_V5R1_EXTENSIONS_KEY_BITS),
        }
    }
}

impl TlbSerialize for WalletV5R1Data {
    fn store_tlb(&self, builder: &mut Builder) -> crate::tlb::Result<()> {
        if self.extensions.key_bits() != WALLET_V5R1_EXTENSIONS_KEY_BITS {
            return Err(TlbError::CustomSchema {
                schema: "WalletV5R1Data.extensions",
                message: format!(
                    "extension dictionary key width {} is not 256",
                    self.extensions.key_bits()
                ),
            });
        }

        builder.store_bit(self.is_signature_allowed)?;
        builder.store_u32(self.seqno)?;
        builder.store_u32(self.wallet_id)?;
        builder.store_bytes(&self.public_key)?;
        builder.store_hashmap_e_with(&self.extensions, |builder, value| {
            builder.store_bit(*value)?;
            Ok(())
        })?;
        Ok(())
    }
}

impl TlbDeserialize for WalletV5R1Data {
    fn load_tlb(slice: &mut Slice) -> crate::tlb::Result<Self> {
        Ok(Self {
            is_signature_allowed: slice.load_bit()?,
            seqno: slice.load_u32()?,
            wallet_id: slice.load_u32()?,
            public_key: {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&slice.load_bytes(32)?);
                bytes
            },
            extensions: slice
                .load_hashmap_e_with(WALLET_V5R1_EXTENSIONS_KEY_BITS, |slice| slice.load_bit())?,
        })
    }
}

/// Persistent Wallet V4R2 storage data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV4R2Data {
    /// Current wallet sequence number.
    pub seqno: u32,
    /// V4 wallet id, historically named `subwallet_id`.
    pub wallet_id: u32,
    /// Ed25519 public key bytes.
    pub public_key: [u8; 32],
    /// Plugin dictionary keyed by 256-bit address hash, with `int1` values.
    pub plugins: HashmapE<bool>,
}

impl WalletV4R2Data {
    /// Creates initial Wallet V4R2 data with seqno zero and empty plugins.
    pub fn new(wallet_id: u32, public_key: [u8; 32]) -> Self {
        Self {
            seqno: 0,
            wallet_id,
            public_key,
            plugins: HashmapE::new(WALLET_V4R2_PLUGINS_KEY_BITS),
        }
    }
}

impl TlbSerialize for WalletV4R2Data {
    fn store_tlb(&self, builder: &mut Builder) -> crate::tlb::Result<()> {
        if self.plugins.key_bits() != WALLET_V4R2_PLUGINS_KEY_BITS {
            return Err(TlbError::CustomSchema {
                schema: "WalletV4R2Data.plugins",
                message: format!(
                    "plugin dictionary key width {} is not 256",
                    self.plugins.key_bits()
                ),
            });
        }

        builder.store_u32(self.seqno)?;
        builder.store_u32(self.wallet_id)?;
        builder.store_bytes(&self.public_key)?;
        builder.store_hashmap_e_with(&self.plugins, |builder, value| {
            builder.store_bit(*value)?;
            Ok(())
        })?;
        Ok(())
    }
}

impl TlbDeserialize for WalletV4R2Data {
    fn load_tlb(slice: &mut Slice) -> crate::tlb::Result<Self> {
        Ok(Self {
            seqno: slice.load_u32()?,
            wallet_id: slice.load_u32()?,
            public_key: {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&slice.load_bytes(32)?);
                bytes
            },
            plugins: slice
                .load_hashmap_e_with(WALLET_V4R2_PLUGINS_KEY_BITS, |slice| slice.load_bit())?,
        })
    }
}

/// Returns the embedded official Wallet V4R2 code cell.
pub fn wallet_v4r2_code() -> Result<Arc<Cell>, WalletError> {
    cached_wallet_code(&WALLET_V4R2_CODE, decode_wallet_v4r2_code)
}

/// Returns the embedded official Wallet V5R1 code cell.
pub fn wallet_v5r1_code() -> Result<Arc<Cell>, WalletError> {
    cached_wallet_code(&WALLET_V5R1_CODE, decode_wallet_v5r1_code)
}

fn cached_wallet_code(
    cache: &'static OnceLock<Result<Arc<Cell>, String>>,
    decode: fn() -> Result<Arc<Cell>, WalletError>,
) -> Result<Arc<Cell>, WalletError> {
    match cache.get_or_init(|| decode().map_err(|error| error.to_string())) {
        Ok(cell) => Ok(cell.clone()),
        Err(error) => Err(WalletError::CodeDecode(error.clone())),
    }
}

fn decode_wallet_v4r2_code() -> Result<Arc<Cell>, WalletError> {
    let mut bytes = hex::decode(WALLET_V4R2_CODE_BOC_HEX)
        .map_err(|error| WalletError::CodeDecode(error.to_string()))?;
    strip_boc_crc32c_for_local_decoder(&mut bytes);
    crate::tvm::deserialize_boc(&bytes).map_err(WalletError::Tvm)
}

fn decode_wallet_v5r1_code() -> Result<Arc<Cell>, WalletError> {
    let mut bytes = hex::decode(WALLET_V5R1_CODE_BOC_HEX)
        .map_err(|error| WalletError::CodeDecode(error.to_string()))?;
    strip_boc_crc32c_for_local_decoder(&mut bytes);
    crate::tvm::deserialize_boc(&bytes).map_err(WalletError::Tvm)
}

fn strip_boc_crc32c_for_local_decoder(bytes: &mut Vec<u8>) {
    if bytes.len() >= 9
        && bytes[0..4] == [0xb5, 0xee, 0x9c, 0x72]
        && (bytes[4] & 0x40) != 0
        && bytes.len() >= 4
    {
        bytes[4] &= !0x40;
        bytes.truncate(bytes.len() - 4);
    }
}

/// Wallet V5R1 wallet-id context before XOR with the network global id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletV5R1Context {
    /// Client context: `context_id_client$1 wc:int8 wallet_version:uint8 counter:uint15`.
    Client {
        /// Signed 8-bit workchain.
        workchain: i8,
        /// Wallet-version byte. For V5R1 defaults this is currently `0`.
        wallet_version: u8,
        /// Fifteen-bit subwallet counter.
        subwallet_number: u16,
    },
    /// Custom backoffice context: `context_id_backoffice$0 counter:uint31`.
    Custom(u32),
}

/// Wallet V5R1 id helper that preserves the signed network global id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletV5R1WalletId {
    /// Network global id, for example `-239` for mainnet or `-3` for testnet.
    pub network_global_id: i32,
    /// Context that is XORed with `network_global_id`.
    pub context: WalletV5R1Context,
}

impl WalletV5R1WalletId {
    /// Creates a client-context wallet id helper.
    pub fn client(
        network_global_id: i32,
        workchain: i8,
        wallet_version: u8,
        subwallet_number: u16,
    ) -> Self {
        Self {
            network_global_id,
            context: WalletV5R1Context::Client {
                workchain,
                wallet_version,
                subwallet_number,
            },
        }
    }

    /// Creates the default mainnet V5R1 wallet id helper.
    pub fn mainnet_default(workchain: i8) -> Self {
        Self::client(MAINNET_GLOBAL_ID, workchain, 0, 0)
    }

    /// Creates the default testnet V5R1 wallet id helper.
    pub fn testnet_default(workchain: i8) -> Self {
        Self::client(TESTNET_GLOBAL_ID, workchain, 0, 0)
    }

    /// Unpacks a raw wallet id using the known network global id.
    pub fn unpack(wallet_id: u32, network_global_id: i32) -> Self {
        let context_id = wallet_id ^ network_global_id as u32;
        let context = if context_id & WALLET_V5R1_CLIENT_CONTEXT_FLAG != 0 {
            WalletV5R1Context::Client {
                workchain: ((context_id >> 23) as u8) as i8,
                wallet_version: (context_id >> 15) as u8,
                subwallet_number: (context_id & WALLET_V5R1_CLIENT_SUBWALLET_MAX as u32) as u16,
            }
        } else {
            WalletV5R1Context::Custom(context_id)
        };
        Self {
            network_global_id,
            context,
        }
    }

    /// Packs this helper into the raw 32-bit `wallet_id` stored by the contract.
    pub fn pack(&self) -> Result<u32, WalletError> {
        let context_id = match self.context {
            WalletV5R1Context::Client {
                workchain,
                wallet_version,
                subwallet_number,
            } => {
                if subwallet_number as u32 > WALLET_V5R1_CLIENT_SUBWALLET_MAX as u32 {
                    return Err(WalletError::SubwalletNumberTooLarge(
                        subwallet_number as u32,
                    ));
                }
                WALLET_V5R1_CLIENT_CONTEXT_FLAG
                    | ((workchain as u8 as u32) << 23)
                    | ((wallet_version as u32) << 15)
                    | subwallet_number as u32
            }
            WalletV5R1Context::Custom(value) => {
                if value & WALLET_V5R1_CLIENT_CONTEXT_FLAG != 0 {
                    return Err(WalletError::CustomContextTooLarge(value));
                }
                value
            }
        };
        Ok(self.network_global_id as u32 ^ context_id)
    }
}

/// A Wallet V5R1 outbound internal message action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletMessage {
    /// Send mode used by `action_send_msg`.
    pub mode: u8,
    /// Destination internal address.
    pub destination: Address,
    /// Native TON amount in nanotons.
    pub value: u64,
    /// Optional body cell, stored inline when present.
    pub body: Option<Arc<Cell>>,
    /// Whether the internal message should bounce.
    pub bounce: bool,
}

impl WalletMessage {
    /// Creates a simple internal transfer action.
    pub fn internal(destination: Address, value: u64) -> Self {
        Self {
            mode: 3,
            destination,
            value,
            body: None,
            bounce: true,
        }
    }

    /// Sets the send mode.
    pub fn with_mode(mut self, mode: u8) -> Self {
        self.mode = mode;
        self
    }

    /// Sets an inline body cell.
    pub fn with_body(mut self, body: Arc<Cell>) -> Self {
        self.body = Some(body);
        self
    }

    /// Sets the bounce flag.
    pub fn with_bounce(mut self, bounce: bool) -> Self {
        self.bounce = bounce;
        self
    }

    fn into_message_relaxed(self) -> MessageRelaxed {
        let body = self
            .body
            .unwrap_or_else(|| Builder::new().build().expect("empty cell builds"));
        MessageRelaxed {
            info: CommonMsgInfoRelaxed::Internal {
                ihr_disabled: true,
                bounce: self.bounce,
                bounced: false,
                src: MsgAddress::Ext(MsgAddressExt::None),
                dest: MsgAddressInt::std(self.destination),
                value: CurrencyCollection::grams(Grams(BigUint::from(self.value))),
                extra_flags: BigUint::from(0u8),
                fwd_fee: Grams(BigUint::from(0u8)),
                created_lt: 0,
                created_at: 0,
            },
            init: None,
            body: Either::Left(body),
        }
    }

    fn into_action(self) -> OutAction {
        OutAction::SendMsg {
            mode: self.mode,
            out_msg: self.into_message_relaxed(),
        }
    }
}

/// Wallet V4R2 offline helper bound to code, workchain, wallet id, and public key.
#[derive(Debug, Clone)]
pub struct WalletV4R2 {
    workchain: i8,
    wallet_id: u32,
    public_key: [u8; 32],
    code: Arc<Cell>,
}

impl WalletV4R2 {
    /// Creates a Wallet V4R2 helper from a public key, raw wallet id, code cell,
    /// and workchain.
    pub fn new(public_key: [u8; 32], wallet_id: u32, code: Arc<Cell>, workchain: i8) -> Self {
        Self {
            workchain,
            wallet_id,
            public_key,
            code,
        }
    }

    /// Creates a Wallet V4R2 helper with the common default wallet id.
    pub fn default(public_key: [u8; 32], code: Arc<Cell>, workchain: i8) -> Self {
        Self::new(public_key, WALLET_V4R2_DEFAULT_ID, code, workchain)
    }

    /// Returns the wallet workchain.
    pub fn workchain(&self) -> i8 {
        self.workchain
    }

    /// Returns the raw 32-bit wallet id.
    pub fn wallet_id(&self) -> u32 {
        self.wallet_id
    }

    /// Returns the configured public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Builds the initial data cell value.
    pub fn data(&self) -> WalletV4R2Data {
        WalletV4R2Data::new(self.wallet_id, self.public_key)
    }

    /// Builds the wallet `StateInit`.
    pub fn state_init(&self) -> Result<StateInit, WalletError> {
        Ok(StateInit {
            code: Some(self.code.clone()),
            data: Some(self.data().to_cell()?),
            ..StateInit::empty()
        })
    }

    /// Derives the wallet address from `StateInit`.
    pub fn address(&self) -> Result<Address, WalletError> {
        let state_init = self.state_init()?;
        Ok(Address::new(self.workchain, state_init.to_cell()?.hash()))
    }

    /// Builds the unsigned signing cell for a Wallet V4R2 simple send.
    pub fn build_external_signing_cell(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
    ) -> Result<Arc<Cell>, WalletError> {
        if messages.len() > WALLET_V4R2_MAX_MESSAGES {
            return Err(WalletError::TooManyActions {
                count: messages.len(),
                max: WALLET_V4R2_MAX_MESSAGES,
            });
        }

        let mut builder = Builder::new();
        builder.store_u32(self.wallet_id)?;
        builder.store_u32(valid_until)?;
        builder.store_u32(seqno)?;
        builder.store_u32(WALLET_V4R2_SIMPLE_SEND_OP)?;
        for message in messages {
            let mode = message.mode;
            let relaxed = message.into_message_relaxed();
            builder.store_u8(mode)?;
            builder.store_ref(relaxed.to_cell()?)?;
        }
        Ok(builder.build()?)
    }

    /// Builds a signed external body cell and returns the body, signed hash,
    /// and Ed25519 signature.
    pub fn build_signed_external_body(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
    ) -> Result<WalletV4R2SignedBody, WalletError> {
        let signing_cell = self.build_external_signing_cell(seqno, valid_until, messages)?;
        let signing_hash = signing_cell.hash();
        let signature = signing_key.sign(&signing_hash).to_bytes();

        let mut builder = Builder::new();
        builder.store_bytes(&signature)?;
        builder.store_cell(&signing_cell)?;
        Ok(WalletV4R2SignedBody {
            body: builder.build()?,
            signing_hash,
            signature,
        })
    }

    /// Builds an external inbound message BoC with the signed body.
    pub fn build_external_message_boc(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        let signed = self.build_signed_external_body(seqno, valid_until, messages, signing_key)?;
        let state_init = if include_state_init {
            Some(Either::Right(self.state_init()?))
        } else {
            None
        };
        let message = Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: MsgAddressInt::std(self.address()?),
                import_fee: Grams(BigUint::from(0u8)),
            },
            init: state_init,
            body: Either::Right(signed.body),
        };
        Ok(serialize_boc(&message.to_cell()?, false)?)
    }

    /// Sends a signed external message BoC through any contract provider.
    #[cfg(feature = "liteclient")]
    pub async fn send_external_message<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<u32, WalletSendError<P::Error>> {
        let boc = self
            .build_external_message_boc(
                seqno,
                valid_until,
                messages,
                signing_key,
                include_state_init,
            )
            .map_err(WalletSendError::Build)?;
        provider
            .send_external_message_boc(boc)
            .await
            .map_err(WalletSendError::Provider)
    }
}

/// Signed Wallet V4R2 external body material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV4R2SignedBody {
    /// Final body cell containing signature followed by signing fields.
    pub body: Arc<Cell>,
    /// Representation hash that was signed.
    pub signing_hash: [u8; 32],
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
}

/// Wallet V5R1 offline helper bound to code, workchain, wallet id, and public key.
#[derive(Debug, Clone)]
pub struct WalletV5R1 {
    workchain: i8,
    wallet_id: u32,
    public_key: [u8; 32],
    code: Arc<Cell>,
    is_signature_allowed: bool,
}

impl WalletV5R1 {
    /// Creates a Wallet V5R1 helper from a public key, raw wallet id, code cell,
    /// and workchain.
    pub fn new(public_key: [u8; 32], wallet_id: u32, code: Arc<Cell>, workchain: i8) -> Self {
        Self {
            workchain,
            wallet_id,
            public_key,
            code,
            is_signature_allowed: true,
        }
    }

    /// Creates a Wallet V5R1 helper from a packed wallet-id helper.
    pub fn from_wallet_id(
        public_key: [u8; 32],
        wallet_id: WalletV5R1WalletId,
        code: Arc<Cell>,
        workchain: i8,
    ) -> Result<Self, WalletError> {
        Ok(Self::new(public_key, wallet_id.pack()?, code, workchain))
    }

    /// Returns the wallet workchain.
    pub fn workchain(&self) -> i8 {
        self.workchain
    }

    /// Returns the raw 32-bit wallet id.
    pub fn wallet_id(&self) -> u32 {
        self.wallet_id
    }

    /// Returns the configured public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// Builds the initial data cell value.
    pub fn data(&self) -> WalletV5R1Data {
        WalletV5R1Data {
            is_signature_allowed: self.is_signature_allowed,
            seqno: 0,
            wallet_id: self.wallet_id,
            public_key: self.public_key,
            extensions: HashmapE::new(WALLET_V5R1_EXTENSIONS_KEY_BITS),
        }
    }

    /// Builds the wallet `StateInit`.
    pub fn state_init(&self) -> Result<StateInit, WalletError> {
        Ok(StateInit {
            code: Some(self.code.clone()),
            data: Some(self.data().to_cell()?),
            ..StateInit::empty()
        })
    }

    /// Derives the wallet address from `StateInit`.
    pub fn address(&self) -> Result<Address, WalletError> {
        let state_init = self.state_init()?;
        Ok(Address::new(self.workchain, state_init.to_cell()?.hash()))
    }

    /// Builds the unsigned signing cell for an external signed request.
    pub fn build_external_signing_cell(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
    ) -> Result<Arc<Cell>, WalletError> {
        validate_action_count(messages.len())?;
        let out_list = if messages.is_empty() {
            None
        } else {
            Some(OutList::new(
                messages
                    .into_iter()
                    .map(WalletMessage::into_action)
                    .collect(),
            ))
        };

        let mut builder = Builder::new();
        builder.store_u32(WALLET_V5R1_EXTERNAL_SIGNED_OP)?;
        builder.store_u32(self.wallet_id)?;
        builder.store_u32(valid_until)?;
        builder.store_u32(seqno)?;
        store_v5_inner_request(&mut builder, out_list.as_ref())?;
        Ok(builder.build()?)
    }

    /// Builds a signed external body cell and returns the body, signed hash, and
    /// Ed25519 signature.
    pub fn build_signed_external_body(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
    ) -> Result<WalletV5R1SignedBody, WalletError> {
        let signing_cell = self.build_external_signing_cell(seqno, valid_until, messages)?;
        let signing_hash = signing_cell.hash();
        let signature = signing_key.sign(&signing_hash).to_bytes();

        let mut builder = Builder::new();
        builder.store_cell(&signing_cell)?;
        builder.store_bytes(&signature)?;
        Ok(WalletV5R1SignedBody {
            body: builder.build()?,
            signing_hash,
            signature,
        })
    }

    /// Builds an external inbound message BoC with the signed body.
    pub fn build_external_message_boc(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        let signed = self.build_signed_external_body(seqno, valid_until, messages, signing_key)?;
        let state_init = if include_state_init {
            Some(Either::Right(self.state_init()?))
        } else {
            None
        };
        let message = Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: MsgAddressInt::std(self.address()?),
                import_fee: Grams(BigUint::from(0u8)),
            },
            init: state_init,
            body: Either::Right(signed.body),
        };
        Ok(serialize_boc(&message.to_cell()?, false)?)
    }

    /// Sends a signed external message BoC through any contract provider.
    #[cfg(feature = "liteclient")]
    pub async fn send_external_message<P: crate::contracts::ContractProvider + ?Sized>(
        &self,
        provider: &mut P,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<u32, WalletSendError<P::Error>> {
        let boc = self
            .build_external_message_boc(
                seqno,
                valid_until,
                messages,
                signing_key,
                include_state_init,
            )
            .map_err(WalletSendError::Build)?;
        provider
            .send_external_message_boc(boc)
            .await
            .map_err(WalletSendError::Provider)
    }
}

/// Signed Wallet V5R1 external body material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV5R1SignedBody {
    /// Final body cell containing signing fields and signature.
    pub body: Arc<Cell>,
    /// Representation hash that was signed.
    pub signing_hash: [u8; 32],
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
}

/// Decoded Wallet V5R1 external signed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletV5R1ExternalBody {
    /// Wallet id from the signed request.
    pub wallet_id: u32,
    /// Expiration unix timestamp.
    pub valid_until: u32,
    /// Message seqno.
    pub seqno: u32,
    /// Standard outgoing actions, when present.
    pub out_list: Option<OutList>,
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
}

impl WalletV5R1ExternalBody {
    /// Decodes a signed Wallet V5R1 external body cell.
    pub fn from_cell(cell: Arc<Cell>) -> crate::tlb::Result<Self> {
        let signature_bit_len = 512;
        if cell.bit_len() < signature_bit_len {
            return Err(TlbError::CustomSchema {
                schema: "WalletV5R1ExternalBody",
                message: "body is shorter than the 512-bit signature".to_string(),
            });
        }
        let mut slice = Slice::new(cell);
        let op = slice.load_u32()?;
        if op != WALLET_V5R1_EXTERNAL_SIGNED_OP {
            return Err(TlbError::CustomSchema {
                schema: "WalletV5R1ExternalBody",
                message: format!("unexpected op 0x{op:08x}"),
            });
        }
        let wallet_id = slice.load_u32()?;
        let valid_until = slice.load_u32()?;
        let seqno = slice.load_u32()?;
        let out_list = load_v5_inner_request(&mut slice)?;
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&slice.load_bytes(64)?);
        ensure_empty(&slice)?;
        Ok(Self {
            wallet_id,
            valid_until,
            seqno,
            out_list,
            signature,
        })
    }
}

fn validate_action_count(count: usize) -> Result<(), WalletError> {
    if count > WALLET_V5R1_MAX_ACTIONS {
        return Err(WalletError::TooManyActions {
            count,
            max: WALLET_V5R1_MAX_ACTIONS,
        });
    }
    Ok(())
}

fn store_v5_inner_request(
    builder: &mut Builder,
    out_list: Option<&OutList>,
) -> crate::tlb::Result<()> {
    match out_list {
        Some(list) => {
            builder.store_bit(true)?;
            builder.store_ref(list.to_cell()?)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    builder.store_bit(false)?;
    Ok(())
}

fn load_v5_inner_request(slice: &mut Slice) -> crate::tlb::Result<Option<OutList>> {
    let out_list = if slice.load_bit()? {
        Some(crate::tlb::load_ref_tlb(
            slice,
            "WalletV5R1ExternalBody.out_list",
        )?)
    } else {
        None
    };
    if slice.load_bit()? {
        return Err(TlbError::CustomSchema {
            schema: "WalletV5R1ExternalBody.extended_actions",
            message: "extended actions are not decoded by the initial Wallet V5R1 helper"
                .to_string(),
        });
    }
    Ok(out_list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tlb::{TlbDeserialize, TlbSerialize};
    use crate::tvm::deserialize_boc;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    fn test_code() -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u32(0xfeed_beef).unwrap();
        builder.build().unwrap()
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn fixture_mnemonic() -> &'static str {
        "open price dish charge law skirt alien churn fire swap number brass outdoor diamond lesson april remain puzzle title elbow valley grant champion staff"
    }

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
        assert_eq!(decoded.signature, signed.signature);
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
}
