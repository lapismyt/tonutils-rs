//! TEP-74 jetton message body builders.

use crate::tlb::{
    CellRef, Either, Grams, MsgAddress, MsgAddressInt, Result, TlbDeserialize, TlbSerialize,
};
use crate::tvm::{Address, Builder, Cell, Slice};
use num_bigint::BigUint;
use std::sync::Arc;

/// TEP-74 `transfer` operation code.
pub const JETTON_TRANSFER_OP: u32 = 0x0f8a_7ea5;
/// TEP-74 `transfer_notification` operation code.
pub const JETTON_TRANSFER_NOTIFICATION_OP: u32 = 0x7362_d09c;
/// TEP-74 `internal_transfer` operation code.
pub const JETTON_INTERNAL_TRANSFER_OP: u32 = 0x178d_4519;
/// TEP-74 `burn` operation code.
pub const JETTON_BURN_OP: u32 = 0x595f_07bc;
/// TEP-74 `excesses` operation code.
pub const JETTON_EXCESSES_OP: u32 = 0xd532_76db;

/// Raw forward payload branch used by jetton and NFT message bodies.
pub type ForwardPayload = Either<Arc<Cell>, CellRef<Arc<Cell>>>;

/// TEP-74 `transfer` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonTransferPayload {
    pub query_id: u64,
    pub amount: BigUint,
    pub destination: MsgAddress,
    pub response_destination: MsgAddress,
    pub custom_payload: Option<CellRef<Arc<Cell>>>,
    pub forward_ton_amount: BigUint,
    pub forward_payload: ForwardPayload,
}

impl JettonTransferPayload {
    /// Creates a transfer between standard internal addresses.
    pub fn new(
        query_id: u64,
        amount: impl Into<BigUint>,
        destination: Address,
        response_destination: Address,
    ) -> Self {
        Self {
            query_id,
            amount: amount.into(),
            destination: std_address(destination),
            response_destination: std_address(response_destination),
            custom_payload: None,
            forward_ton_amount: BigUint::from(0u8),
            forward_payload: empty_forward_payload(),
        }
    }

    pub fn with_custom_payload(mut self, payload: Arc<Cell>) -> Self {
        self.custom_payload = Some(CellRef(payload));
        self
    }

    pub fn with_forward_payload(
        mut self,
        forward_ton_amount: impl Into<BigUint>,
        payload: ForwardPayload,
    ) -> Self {
        self.forward_ton_amount = forward_ton_amount.into();
        self.forward_payload = payload;
        self
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonTransferPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(JETTON_TRANSFER_OP)?;
        builder.store_u64(self.query_id)?;
        grams(&self.amount).store_tlb(builder)?;
        self.destination.store_tlb(builder)?;
        self.response_destination.store_tlb(builder)?;
        self.custom_payload.store_tlb(builder)?;
        grams(&self.forward_ton_amount).store_tlb(builder)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for JettonTransferPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(slice, JETTON_TRANSFER_OP, "JettonTransferPayload")?;
        Ok(Self {
            query_id: slice.load_u64()?,
            amount: Grams::load_tlb(slice)?.0,
            destination: MsgAddress::load_tlb(slice)?,
            response_destination: MsgAddress::load_tlb(slice)?,
            custom_payload: Option::<CellRef<Arc<Cell>>>::load_tlb(slice)?,
            forward_ton_amount: Grams::load_tlb(slice)?.0,
            forward_payload: ForwardPayload::load_tlb(slice)?,
        })
    }
}

/// TEP-74 `burn` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonBurnPayload {
    pub query_id: u64,
    pub amount: BigUint,
    pub response_destination: MsgAddress,
    pub custom_payload: Option<CellRef<Arc<Cell>>>,
}

impl JettonBurnPayload {
    pub fn new(query_id: u64, amount: impl Into<BigUint>, response_destination: Address) -> Self {
        Self {
            query_id,
            amount: amount.into(),
            response_destination: std_address(response_destination),
            custom_payload: None,
        }
    }

    pub fn with_custom_payload(mut self, payload: Arc<Cell>) -> Self {
        self.custom_payload = Some(CellRef(payload));
        self
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonBurnPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(JETTON_BURN_OP)?;
        builder.store_u64(self.query_id)?;
        grams(&self.amount).store_tlb(builder)?;
        self.response_destination.store_tlb(builder)?;
        self.custom_payload.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for JettonBurnPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(slice, JETTON_BURN_OP, "JettonBurnPayload")?;
        Ok(Self {
            query_id: slice.load_u64()?,
            amount: Grams::load_tlb(slice)?.0,
            response_destination: MsgAddress::load_tlb(slice)?,
            custom_payload: Option::<CellRef<Arc<Cell>>>::load_tlb(slice)?,
        })
    }
}

/// TEP-74 `internal_transfer` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonInternalTransferPayload {
    pub query_id: u64,
    pub amount: BigUint,
    pub from: MsgAddress,
    pub response_address: MsgAddress,
    pub forward_ton_amount: BigUint,
    pub forward_payload: ForwardPayload,
}

impl JettonInternalTransferPayload {
    pub fn new(
        query_id: u64,
        amount: impl Into<BigUint>,
        from: Address,
        response_address: Address,
    ) -> Self {
        Self {
            query_id,
            amount: amount.into(),
            from: std_address(from),
            response_address: std_address(response_address),
            forward_ton_amount: BigUint::from(0u8),
            forward_payload: empty_forward_payload(),
        }
    }

    pub fn with_forward_payload(
        mut self,
        forward_ton_amount: impl Into<BigUint>,
        payload: ForwardPayload,
    ) -> Self {
        self.forward_ton_amount = forward_ton_amount.into();
        self.forward_payload = payload;
        self
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonInternalTransferPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(JETTON_INTERNAL_TRANSFER_OP)?;
        builder.store_u64(self.query_id)?;
        grams(&self.amount).store_tlb(builder)?;
        self.from.store_tlb(builder)?;
        self.response_address.store_tlb(builder)?;
        grams(&self.forward_ton_amount).store_tlb(builder)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for JettonInternalTransferPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(
            slice,
            JETTON_INTERNAL_TRANSFER_OP,
            "JettonInternalTransferPayload",
        )?;
        Ok(Self {
            query_id: slice.load_u64()?,
            amount: Grams::load_tlb(slice)?.0,
            from: MsgAddress::load_tlb(slice)?,
            response_address: MsgAddress::load_tlb(slice)?,
            forward_ton_amount: Grams::load_tlb(slice)?.0,
            forward_payload: ForwardPayload::load_tlb(slice)?,
        })
    }
}

/// Builds a `transfer_notification` body.
pub fn jetton_transfer_notification_payload(
    query_id: u64,
    amount: impl Into<BigUint>,
    sender: MsgAddress,
    forward_payload: ForwardPayload,
) -> Result<Arc<Cell>> {
    let mut builder = Builder::new();
    builder.store_u32(JETTON_TRANSFER_NOTIFICATION_OP)?;
    builder.store_u64(query_id)?;
    grams(&amount.into()).store_tlb(&mut builder)?;
    sender.store_tlb(&mut builder)?;
    forward_payload.store_tlb(&mut builder)?;
    Ok(builder.build()?)
}

/// Builds an `excesses` body.
pub fn jetton_excesses_payload(query_id: u64) -> Result<Arc<Cell>> {
    query_id_payload(JETTON_EXCESSES_OP, query_id)
}

pub fn inline_forward_payload(cell: Arc<Cell>) -> ForwardPayload {
    Either::Left(cell)
}

pub fn referenced_forward_payload(cell: Arc<Cell>) -> ForwardPayload {
    Either::Right(CellRef(cell))
}

pub fn empty_forward_payload() -> ForwardPayload {
    inline_forward_payload(Builder::new().build().expect("empty cell builds"))
}

pub(crate) fn std_address(address: Address) -> MsgAddress {
    MsgAddress::Int(MsgAddressInt::std(address))
}

pub(crate) fn query_id_payload(op: u32, query_id: u64) -> Result<Arc<Cell>> {
    let mut builder = Builder::new();
    builder.store_u32(op)?;
    builder.store_u64(query_id)?;
    Ok(builder.build()?)
}

fn grams(value: &BigUint) -> Grams {
    Grams(value.clone())
}

fn expect_op(slice: &mut Slice, expected: u32, schema: &'static str) -> Result<()> {
    let actual = slice.load_u32()?;
    if actual != expected {
        return Err(crate::tlb::TlbError::CustomSchema {
            schema,
            message: format!("operation code 0x{actual:08x} does not match 0x{expected:08x}"),
        });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn exact_from_cell<T: TlbDeserialize>(cell: Arc<Cell>) -> Result<T> {
    let mut slice = Slice::new(cell);
    let value = T::load_tlb(&mut slice)?;
    crate::tlb::ensure_empty(&slice)?;
    Ok(value)
}
