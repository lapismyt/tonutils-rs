//! Outgoing wallet messages, including contract deployment.
//!
//! [`WalletMessage`] covers plain internal transfers. [`WalletOutgoingMessage`]
//! additionally carries an optional `StateInit`, which lets a wallet deploy a
//! contract (jetton, NFT, or any other wrapper implementing
//! [`ContractDefinition`](crate::contracts::ContractDefinition)) with the same
//! signed external message that pays for it.

use std::sync::Arc;

use crate::{
    contracts::{ContractBuildError, DeployRequest, address_from_state_init},
    tlb::{
        CommonMsgInfoRelaxed, CurrencyCollection, Either, Grams, MessageRelaxed, MsgAddress,
        MsgAddressExt, MsgAddressInt, OutAction, StateInit, TlbResult,
    },
    tvm::{Address, Builder, Cell},
};

use super::mnemonic::WalletMessage;

use num_bigint::BigUint;

/// An internal message sent by a wallet, with an optional deploy `StateInit`.
#[derive(Debug, Clone)]
pub struct WalletOutgoingMessage {
    /// Send mode used by `action_send_msg`. Defaults to `3`.
    pub mode: u8,
    /// Destination internal address.
    pub destination: Address,
    /// Native TON amount in nanotons.
    pub value: u64,
    /// Optional body cell.
    pub body: Option<Arc<Cell>>,
    /// Whether the internal message should bounce.
    pub bounce: bool,
    /// Optional contract `StateInit`. When set, the message deploys the
    /// destination if it is not active yet.
    pub state_init: Option<StateInit>,
}

impl WalletOutgoingMessage {
    /// Creates a simple internal transfer action.
    pub fn new(destination: Address, value: u64) -> Self {
        Self {
            mode: 3,
            destination,
            value,
            body: None,
            bounce: true,
            state_init: None,
        }
    }

    /// Creates a deploy message for an already derived destination address.
    pub fn deploy_to(destination: Address, deploy: DeployRequest) -> Self {
        Self {
            mode: deploy.mode,
            destination,
            value: grams_to_nanotons(&deploy.amount),
            body: deploy.body,
            bounce: deploy.bounce,
            state_init: Some(deploy.state_init),
        }
    }

    /// Creates a deploy message, deriving the destination from `StateInit`.
    pub fn deploy(workchain: i8, deploy: DeployRequest) -> Result<Self, ContractBuildError> {
        let destination = address_from_state_init(workchain, &deploy.state_init)
            .map_err(ContractBuildError::StateInitSerialization)?;
        Ok(Self::deploy_to(destination, deploy))
    }

    /// Creates an outgoing message from a plain wallet message.
    pub fn from_wallet_message(message: WalletMessage) -> Self {
        Self {
            mode: message.mode,
            destination: message.destination,
            value: message.value,
            body: message.body,
            bounce: message.bounce,
            state_init: None,
        }
    }

    /// Sets the send mode.
    #[must_use]
    pub fn with_mode(mut self, mode: u8) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the attached TON amount in nanotons.
    #[must_use]
    pub fn with_value(mut self, value: u64) -> Self {
        self.value = value;
        self
    }

    /// Sets the body cell.
    #[must_use]
    pub fn with_body(mut self, body: Arc<Cell>) -> Self {
        self.body = Some(body);
        self
    }

    /// Sets the bounce flag.
    #[must_use]
    pub fn with_bounce(mut self, bounce: bool) -> Self {
        self.bounce = bounce;
        self
    }

    /// Attaches a deploy `StateInit`.
    #[must_use]
    pub fn with_state_init(mut self, state_init: StateInit) -> Self {
        self.state_init = Some(state_init);
        self
    }

    pub(crate) fn into_message_relaxed(self) -> TlbResult<MessageRelaxed> {
        let body = match self.body {
            Some(body) => body,
            None => Builder::new().build()?,
        };
        Ok(MessageRelaxed {
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
            init: self.state_init.map(Either::Left),
            body: Either::Left(body),
        })
    }

    pub(crate) fn into_action(self) -> TlbResult<OutAction> {
        Ok(OutAction::SendMsg {
            mode: self.mode,
            out_msg: self.into_message_relaxed()?,
        })
    }
}

impl From<WalletMessage> for WalletOutgoingMessage {
    fn from(message: WalletMessage) -> Self {
        Self::from_wallet_message(message)
    }
}

impl From<WalletOutgoingMessage> for WalletMessage {
    fn from(message: WalletOutgoingMessage) -> Self {
        Self {
            mode: message.mode,
            destination: message.destination,
            value: message.value,
            body: message.body,
            bounce: message.bounce,
        }
    }
}

fn grams_to_nanotons(amount: &Grams) -> u64 {
    amount
        .0
        .to_u64_digits()
        .first()
        .copied()
        .unwrap_or_default()
}

use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;

use super::message::{WalletV4R2, WalletV4R2SignedBody, WalletV5R1, WalletV5R1SignedBody};
use super::mnemonic::{
    WALLET_V4R2_MAX_MESSAGES, WALLET_V4R2_SIMPLE_SEND_OP, WALLET_V5R1_EXTERNAL_SIGNED_OP,
    WalletError,
};
use super::v5r1::{store_v5_inner_request, validate_action_count};
use crate::tlb::{CommonMsgInfo, Message, OutList, TlbSerialize};
use crate::tvm::serialize_boc;

impl WalletV4R2 {
    /// Builds the unsigned signing cell for outgoing messages that may deploy
    /// contracts.
    pub fn build_external_signing_cell_outgoing(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletOutgoingMessage>,
    ) -> Result<Arc<Cell>, WalletError> {
        if messages.len() > WALLET_V4R2_MAX_MESSAGES {
            return Err(WalletError::TooManyActions {
                count: messages.len(),
                max: WALLET_V4R2_MAX_MESSAGES,
            });
        }

        let mut builder = Builder::new();
        builder.store_u32(self.wallet_id())?;
        builder.store_u32(valid_until)?;
        builder.store_u32(seqno)?;
        builder.store_u32(WALLET_V4R2_SIMPLE_SEND_OP)?;
        for message in messages {
            let mode = message.mode;
            let relaxed = message.into_message_relaxed()?;
            builder.store_u8(mode)?;
            builder.store_ref(relaxed.to_cell()?)?;
        }
        Ok(builder.build()?)
    }

    /// Builds a signed external body from outgoing messages.
    pub fn build_signed_external_body_outgoing(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletOutgoingMessage>,
        signing_key: &SigningKey,
    ) -> Result<WalletV4R2SignedBody, WalletError> {
        let signing_cell =
            self.build_external_signing_cell_outgoing(seqno, valid_until, messages)?;
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

    /// Builds an external inbound message BoC from outgoing messages.
    pub fn build_external_message_boc_outgoing(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletOutgoingMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        let signed =
            self.build_signed_external_body_outgoing(seqno, valid_until, messages, signing_key)?;
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
}

impl WalletV5R1 {
    /// Builds the unsigned signing cell for outgoing messages that may deploy
    /// contracts.
    pub fn build_external_signing_cell_outgoing(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletOutgoingMessage>,
    ) -> Result<Arc<Cell>, WalletError> {
        validate_action_count(messages.len())?;
        let out_list = if messages.is_empty() {
            None
        } else {
            let actions = messages
                .into_iter()
                .map(WalletOutgoingMessage::into_action)
                .collect::<TlbResult<Vec<_>>>()?;
            Some(OutList::new(actions))
        };

        let mut builder = Builder::new();
        builder.store_u32(WALLET_V5R1_EXTERNAL_SIGNED_OP)?;
        builder.store_u32(self.wallet_id())?;
        builder.store_u32(valid_until)?;
        builder.store_u32(seqno)?;
        store_v5_inner_request(&mut builder, out_list.as_ref(), None)?;
        Ok(builder.build()?)
    }

    /// Builds a signed external body from outgoing messages.
    pub fn build_signed_external_body_outgoing(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletOutgoingMessage>,
        signing_key: &SigningKey,
    ) -> Result<WalletV5R1SignedBody, WalletError> {
        let signing_cell =
            self.build_external_signing_cell_outgoing(seqno, valid_until, messages)?;
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

    /// Builds an external inbound message BoC from outgoing messages.
    pub fn build_external_message_boc_outgoing(
        &self,
        seqno: u32,
        valid_until: u32,
        messages: Vec<WalletOutgoingMessage>,
        signing_key: &SigningKey,
        include_state_init: bool,
    ) -> Result<Vec<u8>, WalletError> {
        let signed =
            self.build_signed_external_body_outgoing(seqno, valid_until, messages, signing_key)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ContractDefinition, DeployRequest};
    use crate::tlb::TlbDeserialize;
    use crate::tvm::Slice;

    fn wallet() -> (WalletV4R2, SigningKey) {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let public_key = key.verifying_key().to_bytes();
        let wallet = WalletV4R2::default(
            public_key,
            crate::wallet::wallet_v4r2_code().expect("code decodes"),
            0,
        );
        (wallet, key)
    }

    fn deploy_request() -> DeployRequest {
        let code = Builder::new().build().expect("empty cell builds");
        let mut data_builder = Builder::new();
        data_builder.store_u32(0xabcd_ef01).expect("builder fits");
        let data = data_builder.build().expect("cell builds");
        let state_init = crate::tlb::StateInit {
            code: Some(code),
            data: Some(data),
            ..crate::tlb::StateInit::empty()
        };
        DeployRequest::new(state_init, 500_000_000, None)
    }

    #[test]
    fn deploy_message_derives_destination_and_carries_state_init() {
        let deploy = deploy_request();
        let expected =
            crate::contracts::address_from_state_init(0, &deploy.state_init).expect("address");
        let outgoing = WalletOutgoingMessage::deploy(0, deploy).expect("deploy message");
        assert_eq!(outgoing.destination, expected);
        assert_eq!(outgoing.value, 500_000_000);
        assert!(outgoing.state_init.is_some());

        let relaxed = outgoing.into_message_relaxed().expect("message serializes");
        let cell = relaxed.to_cell().expect("message cell");
        let decoded = crate::tlb::MessageRelaxed::from_cell(cell).expect("message decodes");
        assert!(decoded.init.is_some());
    }

    #[test]
    fn signed_outgoing_body_includes_deploy_state_init() {
        let (wallet, key) = wallet();
        let deploy = WalletOutgoingMessage::deploy(0, deploy_request()).expect("deploy message");
        let cell = wallet
            .build_external_signing_cell_outgoing(0, u32::MAX, vec![deploy])
            .expect("signing cell");
        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_u32().expect("wallet id"), wallet.wallet_id());
        assert_eq!(slice.load_u32().expect("valid until"), u32::MAX);
        assert_eq!(slice.load_u32().expect("seqno"), 0);

        let body = wallet
            .build_signed_external_body_outgoing(0, u32::MAX, Vec::new(), &key)
            .expect("signed body");
        assert_eq!(body.body.bit_len(), 512 + 32 + 32 + 32 + 32);
    }

    #[test]
    fn wallet_implements_contract_definition() {
        let (wallet, _) = wallet();
        assert_eq!(
            <WalletV4R2 as ContractDefinition>::default_code()
                .expect("code decodes")
                .hash(),
            wallet.code().hash()
        );
        assert_eq!(<WalletV4R2 as ContractDefinition>::workchain(&wallet), 0);
        assert_eq!(wallet.data().seqno, 0);
    }
}
