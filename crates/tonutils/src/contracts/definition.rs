//! Shared contract-wrapper instrumentation.
//!
//! A contract wrapper stores its code cell, typed data cell, and workchain. The
//! [`ContractDefinition`] trait exposes those pieces uniformly so applications
//! can compute addresses, build `StateInit` values, and deploy contracts either
//! through a wallet or directly with an internal message.
//!
//! All operations in this module are offline. Live get-methods live in the
//! protocol modules and follow the same address derivation.

use std::sync::Arc;

use crate::{
    tlb::{
        Grams, MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, StateInit, TlbDeserialize,
        TlbError, TlbResult, TlbSerialize,
    },
    tvm::{Address, Builder, Cell, Slice},
};

use super::{
    codes::{ContractCodeError, ContractVersion},
    provider::{ContractBuildError, address_from_state_init},
};

/// Typed contract storage data.
///
/// This trait is blanket-implemented for every type that implements both
/// [`TlbSerialize`] and [`TlbDeserialize`], so protocol data models only need
/// their TL-B codecs.
pub trait ContractData: Sized {
    /// Serializes the data into a standalone cell.
    fn to_cell(&self) -> TlbResult<Arc<Cell>>;

    /// Decodes the data from a standalone cell.
    fn from_cell(cell: &Arc<Cell>) -> TlbResult<Self>;
}

impl<T> ContractData for T
where
    T: TlbSerialize + TlbDeserialize,
{
    fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }

    fn from_cell(cell: &Arc<Cell>) -> TlbResult<Self> {
        let mut slice = Slice::new(cell.clone());
        let value = T::load_tlb(&mut slice)?;
        if !slice.is_empty() {
            return Err(TlbError::TrailingData {
                bits: slice.remaining_bits(),
                refs: slice.remaining_refs(),
            });
        }
        Ok(value)
    }
}

/// A deployable contract definition.
pub trait ContractDefinition {
    /// Typed storage data.
    type Data: ContractData + Clone;

    /// Decodes the embedded default code for this contract edition.
    fn default_code() -> Result<Arc<Cell>, ContractCodeError>;

    /// Returns the contract code cell in use.
    fn code(&self) -> &Arc<Cell>;

    /// Returns the typed contract data.
    fn data(&self) -> Self::Data;

    /// Workchain used for address derivation. Defaults to basechain.
    fn workchain(&self) -> i8 {
        0
    }

    /// Builds the contract `StateInit`.
    fn state_init(&self) -> Result<StateInit, ContractBuildError> {
        build_state_init(self.code(), &self.data())
    }

    /// Computes the contract address offline.
    fn address(&self) -> Result<Address, ContractBuildError> {
        address_from_state_init(self.workchain(), &self.state_init()?)
            .map_err(ContractBuildError::StateInitSerialization)
    }

    /// Builds a deploy request carrying `state_init`, an optional body, and the
    /// amount to attach when the contract is deployed with an internal message.
    fn deploy_request(
        &self,
        amount: u64,
        body: Option<Arc<Cell>>,
    ) -> Result<DeployRequest, ContractBuildError> {
        Ok(DeployRequest {
            state_init: self.state_init()?,
            body,
            amount: Grams::from(amount),
            mode: 3,
            bounce: true,
        })
    }
}

/// Code and data pair used by contract wrappers.
#[derive(Debug, Clone)]
pub struct ContractSpec<D> {
    /// Contract code cell.
    pub code: Arc<Cell>,
    /// Typed storage data.
    pub data: D,
    /// Workchain for address derivation.
    pub workchain: i8,
}

impl<D: ContractData> ContractSpec<D> {
    /// Creates a spec using the embedded default code for `version`.
    pub fn new(version: ContractVersion, data: D) -> Result<Self, ContractCodeError> {
        Ok(Self {
            code: version.default_code()?,
            data,
            workchain: 0,
        })
    }

    /// Creates a spec with explicit code, for custom or upgraded contracts.
    pub fn with_code(code: Arc<Cell>, data: D) -> Self {
        Self {
            code,
            data,
            workchain: 0,
        }
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.workchain = workchain;
        self
    }

    /// Builds the `StateInit` for this spec.
    pub fn state_init(&self) -> Result<StateInit, ContractBuildError> {
        build_state_init(&self.code, &self.data)
    }

    /// Computes the address for this spec.
    pub fn address(&self) -> Result<Address, ContractBuildError> {
        address_from_state_init(self.workchain, &self.state_init()?)
            .map_err(ContractBuildError::StateInitSerialization)
    }
}

/// A ready-to-send contract deployment.
#[derive(Debug, Clone)]
pub struct DeployRequest {
    /// Contract `StateInit`.
    pub state_init: StateInit,
    /// Optional deploy body, for example an initial mint message.
    pub body: Option<Arc<Cell>>,
    /// TON amount attached to the deploy message.
    pub amount: Grams,
    /// Send mode. Defaults to `3` (pay fees separately, ignore errors).
    pub mode: u8,
    /// Whether the deploy message should bounce on failure.
    pub bounce: bool,
}

impl DeployRequest {
    /// Creates a deploy request with the default send mode.
    pub fn new(state_init: StateInit, amount: u64, body: Option<Arc<Cell>>) -> Self {
        Self {
            state_init,
            body,
            amount: Grams::from(amount),
            mode: 3,
            bounce: true,
        }
    }

    /// Converts the request into a relaxed internal message.
    pub fn to_internal_message(&self, destination: MsgAddressInt) -> TlbResult<MessageRelaxed> {
        internal_message(
            destination,
            self.amount.clone(),
            self.body.clone(),
            Some(self.state_init.clone()),
            self.bounce,
        )
    }
}

/// Forward payload accepted by jetton and NFT transfer bodies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ForwardPayload {
    /// No payload (`Either 0`).
    #[default]
    None,
    /// Text comment body (`op=0` followed by UTF-8 bytes).
    Comment(String),
    /// Arbitrary payload cell stored by reference.
    Cell(Arc<Cell>),
}

impl ForwardPayload {
    /// Builds a comment payload.
    pub fn comment(text: impl Into<String>) -> Self {
        Self::Comment(text.into())
    }

    /// Builds a payload from a cell.
    pub fn cell(cell: Arc<Cell>) -> Self {
        Self::Cell(cell)
    }

    /// Serializes the payload cell, when one is present.
    pub fn to_cell(&self) -> TlbResult<Option<Arc<Cell>>> {
        match self {
            Self::None => Ok(None),
            Self::Comment(text) => {
                let mut builder = Builder::new();
                builder.store_u32(0)?;
                builder.store_bytes(text.as_bytes())?;
                Ok(Some(builder.build()?))
            }
            Self::Cell(cell) => Ok(Some(cell.clone())),
        }
    }

    /// Stores the TL-B `Either 0 ^Cell` representation.
    pub fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_maybe_ref(self.to_cell()?)?;
        Ok(())
    }
}

/// Builds a `StateInit` from code and typed data.
pub fn build_state_init<D: ContractData>(
    code: &Arc<Cell>,
    data: &D,
) -> Result<StateInit, ContractBuildError> {
    Ok(StateInit {
        code: Some(code.clone()),
        data: Some(
            data.to_cell()
                .map_err(ContractBuildError::DataSerialization)?,
        ),
        ..StateInit::empty()
    })
}

/// Stores an optional standard address as `MsgAddress`.
pub fn store_address_field(builder: &mut Builder, address: Option<&Address>) -> TlbResult<()> {
    match address {
        Some(address) => MsgAddressInt::std(address.clone()).store_tlb(builder),
        None => MsgAddressExt::None.store_tlb(builder),
    }
}

/// Loads an optional standard address from `MsgAddress`.
pub fn load_address_field(slice: &mut Slice) -> TlbResult<Option<Address>> {
    match MsgAddress::load_tlb(slice)? {
        MsgAddress::Ext(MsgAddressExt::None) => Ok(None),
        MsgAddress::Int(MsgAddressInt::Std {
            anycast: None,
            address,
        }) => Ok(Some(address)),
        MsgAddress::Int(MsgAddressInt::Std {
            anycast: Some(_), ..
        }) => Err(TlbError::NonCanonicalValue {
            schema: "MsgAddress",
            reason: "anycast is not supported in contract data".to_string(),
        }),
        MsgAddress::Int(MsgAddressInt::Var { .. }) => Err(TlbError::NonCanonicalValue {
            schema: "MsgAddress",
            reason: "variable-length addresses are not supported in contract data".to_string(),
        }),
        MsgAddress::Ext(_) => Err(TlbError::NonCanonicalValue {
            schema: "MsgAddress",
            reason: "external addresses are not supported in contract data".to_string(),
        }),
    }
}

/// Builds a relaxed internal message.
pub fn internal_message(
    destination: MsgAddressInt,
    amount: Grams,
    body: Option<Arc<Cell>>,
    state_init: Option<StateInit>,
    bounce: bool,
) -> TlbResult<MessageRelaxed> {
    use crate::tlb::{CommonMsgInfoRelaxed, CurrencyCollection, Either};
    use num_bigint::BigUint;

    Ok(MessageRelaxed {
        info: CommonMsgInfoRelaxed::Internal {
            ihr_disabled: true,
            bounce,
            bounced: false,
            src: MsgAddress::Ext(MsgAddressExt::None),
            dest: destination,
            value: CurrencyCollection::grams(amount),
            extra_flags: BigUint::from(0u8),
            fwd_fee: Grams(BigUint::from(0u8)),
            created_lt: 0,
            created_at: 0,
        },
        init: state_init.map(Either::Left),
        body: Either::Left(
            body.unwrap_or_else(|| Builder::new().build().expect("empty cell always builds")),
        ),
    })
}
