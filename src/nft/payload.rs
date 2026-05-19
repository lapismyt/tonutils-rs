//! TEP-62 and TEP-66 NFT message body builders.

use crate::jetton::{ForwardPayload, empty_forward_payload, query_id_payload, std_address};
use crate::tlb::{CellRef, Grams, MsgAddress, Result, TlbDeserialize, TlbSerialize};
use crate::tvm::{Address, Builder, Cell, Slice};
use num_bigint::BigUint;
use std::sync::Arc;

/// TEP-62 NFT item `transfer` operation code.
pub const NFT_TRANSFER_OP: u32 = 0x5fcc_3d14;
/// TEP-62 NFT item `ownership_assigned` operation code.
pub const NFT_OWNERSHIP_ASSIGNED_OP: u32 = 0x0513_8d91;
/// TEP-62 NFT item `excesses` operation code.
pub const NFT_EXCESSES_OP: u32 = 0xd532_76db;
/// TEP-62 NFT item `get_static_data` operation code.
pub const NFT_GET_STATIC_DATA_OP: u32 = 0x2fcb_26a2;
/// TEP-62 NFT item `report_static_data` operation code.
pub const NFT_REPORT_STATIC_DATA_OP: u32 = 0x8b77_1735;
/// TEP-66 collection `get_royalty_params` operation code.
pub const NFT_GET_ROYALTY_PARAMS_OP: u32 = 0x693d_3950;
/// TEP-66 collection `report_royalty_params` operation code.
pub const NFT_REPORT_ROYALTY_PARAMS_OP: u32 = 0xa8cb_00ad;

/// TEP-62 NFT item `transfer` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftTransferPayload {
    pub query_id: u64,
    pub new_owner: MsgAddress,
    pub response_destination: MsgAddress,
    pub custom_payload: Option<CellRef<Arc<Cell>>>,
    pub forward_amount: BigUint,
    pub forward_payload: ForwardPayload,
}

impl NftTransferPayload {
    /// Creates a transfer between standard internal addresses.
    pub fn new(query_id: u64, new_owner: Address, response_destination: Address) -> Self {
        Self {
            query_id,
            new_owner: std_address(new_owner),
            response_destination: std_address(response_destination),
            custom_payload: None,
            forward_amount: BigUint::from(0u8),
            forward_payload: empty_forward_payload(),
        }
    }

    pub fn with_custom_payload(mut self, payload: Arc<Cell>) -> Self {
        self.custom_payload = Some(CellRef(payload));
        self
    }

    pub fn with_forward_payload(
        mut self,
        forward_amount: impl Into<BigUint>,
        payload: ForwardPayload,
    ) -> Self {
        self.forward_amount = forward_amount.into();
        self.forward_payload = payload;
        self
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftTransferPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(NFT_TRANSFER_OP)?;
        builder.store_u64(self.query_id)?;
        self.new_owner.store_tlb(builder)?;
        self.response_destination.store_tlb(builder)?;
        self.custom_payload.store_tlb(builder)?;
        Grams(self.forward_amount.clone()).store_tlb(builder)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for NftTransferPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(slice, NFT_TRANSFER_OP, "NftTransferPayload")?;
        Ok(Self {
            query_id: slice.load_u64()?,
            new_owner: MsgAddress::load_tlb(slice)?,
            response_destination: MsgAddress::load_tlb(slice)?,
            custom_payload: Option::<CellRef<Arc<Cell>>>::load_tlb(slice)?,
            forward_amount: Grams::load_tlb(slice)?.0,
            forward_payload: ForwardPayload::load_tlb(slice)?,
        })
    }
}

/// TEP-62 NFT item `ownership_assigned` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftOwnershipAssignedPayload {
    pub query_id: u64,
    pub prev_owner: MsgAddress,
    pub forward_payload: ForwardPayload,
}

impl NftOwnershipAssignedPayload {
    pub fn new(query_id: u64, prev_owner: Address, forward_payload: ForwardPayload) -> Self {
        Self {
            query_id,
            prev_owner: std_address(prev_owner),
            forward_payload,
        }
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftOwnershipAssignedPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(NFT_OWNERSHIP_ASSIGNED_OP)?;
        builder.store_u64(self.query_id)?;
        self.prev_owner.store_tlb(builder)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for NftOwnershipAssignedPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(
            slice,
            NFT_OWNERSHIP_ASSIGNED_OP,
            "NftOwnershipAssignedPayload",
        )?;
        Ok(Self {
            query_id: slice.load_u64()?,
            prev_owner: MsgAddress::load_tlb(slice)?,
            forward_payload: ForwardPayload::load_tlb(slice)?,
        })
    }
}

/// TEP-62 NFT item `report_static_data` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftReportStaticDataPayload {
    pub query_id: u64,
    pub index: BigUint,
    pub collection: MsgAddress,
}

impl NftReportStaticDataPayload {
    pub fn new(query_id: u64, index: impl Into<BigUint>, collection: Option<Address>) -> Self {
        Self {
            query_id,
            index: index.into(),
            collection: collection
                .map(std_address)
                .unwrap_or(MsgAddress::Ext(crate::tlb::MsgAddressExt::None)),
        }
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftReportStaticDataPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(NFT_REPORT_STATIC_DATA_OP)?;
        builder.store_u64(self.query_id)?;
        builder.store_big_uint(&self.index, 256)?;
        self.collection.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for NftReportStaticDataPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(
            slice,
            NFT_REPORT_STATIC_DATA_OP,
            "NftReportStaticDataPayload",
        )?;
        Ok(Self {
            query_id: slice.load_u64()?,
            index: slice.load_big_uint(256)?,
            collection: MsgAddress::load_tlb(slice)?,
        })
    }
}

/// TEP-66 `report_royalty_params` message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftReportRoyaltyParamsPayload {
    pub query_id: u64,
    pub numerator: u16,
    pub denominator: u16,
    pub destination: MsgAddress,
}

impl NftReportRoyaltyParamsPayload {
    pub fn new(query_id: u64, numerator: u16, denominator: u16, destination: Address) -> Self {
        Self {
            query_id,
            numerator,
            denominator,
            destination: std_address(destination),
        }
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftReportRoyaltyParamsPayload {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(NFT_REPORT_ROYALTY_PARAMS_OP)?;
        builder.store_u64(self.query_id)?;
        builder.store_u16(self.numerator)?;
        builder.store_u16(self.denominator)?;
        self.destination.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for NftReportRoyaltyParamsPayload {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_op(
            slice,
            NFT_REPORT_ROYALTY_PARAMS_OP,
            "NftReportRoyaltyParamsPayload",
        )?;
        Ok(Self {
            query_id: slice.load_u64()?,
            numerator: slice.load_u16()?,
            denominator: slice.load_u16()?,
            destination: MsgAddress::load_tlb(slice)?,
        })
    }
}

/// Builds a TEP-62 NFT `excesses` message body.
pub fn nft_excesses_payload(query_id: u64) -> Result<Arc<Cell>> {
    query_id_payload(NFT_EXCESSES_OP, query_id)
}

/// Builds a TEP-62 NFT `get_static_data` request body.
pub fn nft_get_static_data_payload(query_id: u64) -> Result<Arc<Cell>> {
    query_id_payload(NFT_GET_STATIC_DATA_OP, query_id)
}

/// Builds a TEP-66 NFT `get_royalty_params` request body.
pub fn nft_get_royalty_params_payload(query_id: u64) -> Result<Arc<Cell>> {
    query_id_payload(NFT_GET_ROYALTY_PARAMS_OP, query_id)
}

#[cfg(test)]
pub(crate) fn exact_from_cell<T: TlbDeserialize>(cell: Arc<Cell>) -> Result<T> {
    let mut slice = Slice::new(cell);
    let value = T::load_tlb(&mut slice)?;
    crate::tlb::ensure_empty(&slice)?;
    Ok(value)
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
