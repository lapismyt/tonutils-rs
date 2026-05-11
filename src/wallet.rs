//! Offline wallet helpers.
//!
//! The first wallet surface is Wallet V5R1. It intentionally starts with
//! deterministic cell construction, address derivation, signing, and external
//! message BoC assembly; live send helpers are thin provider adapters.

use crate::tlb::{
    CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Either, Grams, Message,
    MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, OutAction, OutList, StateInit,
    TlbDeserialize, TlbError, TlbSerialize, ensure_empty,
};
use crate::tvm::{Address, Builder, Cell, HashmapE, Slice, serialize_boc};
use ed25519_dalek::{Signer, SigningKey};
use num_bigint::BigUint;
use std::sync::Arc;

const WALLET_V5R1_EXTERNAL_SIGNED_OP: u32 = 0x7369_676e;
const WALLET_V5R1_EXTENSIONS_KEY_BITS: usize = 256;
const WALLET_V5R1_MAX_ACTIONS: usize = 255;
const WALLET_V5R1_CLIENT_CONTEXT_FLAG: u32 = 1 << 31;
const WALLET_V5R1_CLIENT_SUBWALLET_BITS: u32 = 15;
const WALLET_V5R1_CLIENT_SUBWALLET_MAX: u16 = (1 << WALLET_V5R1_CLIENT_SUBWALLET_BITS) - 1;

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

    fn into_action(self) -> OutAction {
        let body = self
            .body
            .unwrap_or_else(|| Builder::new().build().expect("empty cell builds"));
        OutAction::SendMsg {
            mode: self.mode,
            out_msg: MessageRelaxed {
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
            },
        }
    }
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
}
