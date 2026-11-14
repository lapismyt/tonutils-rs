//! Jetton message bodies.
//!
//! Bodies are plain TL-B payloads; callers attach them to wallet messages or
//! internal deploy messages. See TEP-74 for the operation semantics.

use std::sync::Arc;

use crate::{
    contracts::{ForwardPayload, store_address_field},
    metadata::Tep64Content,
    tlb::{TlbResult, TlbSerialize},
    tvm::{Address, Builder, Cell},
};

/// `transfer#f8a7ea5` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonTransfer {
    /// Query identifier.
    pub query_id: u64,
    /// Amount of jettons to transfer, in base units.
    pub amount: u128,
    /// Recipient owner address.
    pub destination: Address,
    /// Address that receives excess TON.
    pub response_destination: Address,
    /// Optional custom payload forwarded to the sender's wallet.
    pub custom_payload: Option<Arc<Cell>>,
    /// TON amount forwarded with the transfer notification.
    pub forward_ton_amount: u128,
    /// Payload delivered to the recipient.
    pub forward_payload: ForwardPayload,
}

impl JettonTransfer {
    /// Creates a transfer with default query id and no custom payload.
    pub fn new(destination: Address, amount: u128, response_destination: Address) -> Self {
        Self {
            query_id: 0,
            amount,
            destination,
            response_destination,
            custom_payload: None,
            forward_ton_amount: 0,
            forward_payload: ForwardPayload::None,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonTransfer {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x0f8a_7ea5)?;
        builder.store_u64(self.query_id)?;
        builder.store_coins(self.amount)?;
        store_address_field(builder, Some(&self.destination))?;
        store_address_field(builder, Some(&self.response_destination))?;
        builder.store_maybe_ref(self.custom_payload.clone())?;
        builder.store_coins(self.forward_ton_amount)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

/// `internal_transfer#178d4519` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonInternalTransfer {
    /// Query identifier.
    pub query_id: u64,
    /// Amount of jettons, in base units.
    pub amount: u128,
    /// Original sender address.
    pub from: Option<Address>,
    /// Address that receives excess TON.
    pub response_destination: Option<Address>,
    /// TON amount forwarded with the transfer notification.
    pub forward_ton_amount: u128,
    /// Payload delivered to the recipient.
    pub forward_payload: ForwardPayload,
}

impl JettonInternalTransfer {
    /// Creates an internal transfer.
    pub fn new(amount: u128, from: Option<Address>, response_destination: Option<Address>) -> Self {
        Self {
            query_id: 0,
            amount,
            from,
            response_destination,
            forward_ton_amount: 0,
            forward_payload: ForwardPayload::None,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonInternalTransfer {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x178d_4519)?;
        builder.store_u64(self.query_id)?;
        builder.store_coins(self.amount)?;
        store_address_field(builder, self.from.as_ref())?;
        store_address_field(builder, self.response_destination.as_ref())?;
        builder.store_coins(self.forward_ton_amount)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

/// `burn#595f07bc` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonBurn {
    /// Query identifier.
    pub query_id: u64,
    /// Amount of jettons to burn, in base units.
    pub amount: u128,
    /// Address that receives excess TON.
    pub response_destination: Address,
    /// Optional custom payload.
    pub custom_payload: Option<Arc<Cell>>,
}

impl JettonBurn {
    /// Creates a burn body.
    pub fn new(amount: u128, response_destination: Address) -> Self {
        Self {
            query_id: 0,
            amount,
            response_destination,
            custom_payload: None,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonBurn {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x595f_07bc)?;
        builder.store_u64(self.query_id)?;
        builder.store_coins(self.amount)?;
        store_address_field(builder, Some(&self.response_destination))?;
        builder.store_maybe_ref(self.custom_payload.clone())?;
        Ok(())
    }
}

/// Stablecoin `mint#642b7d07` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMint {
    /// Query identifier.
    pub query_id: u64,
    /// Recipient jetton wallet address.
    pub destination: Address,
    /// TON amount forwarded to the destination wallet.
    pub forward_ton_amount: u128,
    /// Internal transfer payload executed by the destination wallet.
    pub internal_transfer: JettonInternalTransfer,
}

impl JettonMint {
    /// Creates a stablecoin mint body.
    pub fn new(
        destination: Address,
        forward_ton_amount: u128,
        internal_transfer: JettonInternalTransfer,
    ) -> Self {
        Self {
            query_id: 0,
            destination,
            forward_ton_amount,
            internal_transfer,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonMint {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x642b_7d07)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, Some(&self.destination))?;
        builder.store_coins(self.forward_ton_amount)?;
        builder.store_ref(self.internal_transfer.to_cell()?)?;
        Ok(())
    }
}

/// Standard `mint#00000015` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonStandardMint {
    /// Query identifier.
    pub query_id: u64,
    /// Recipient jetton wallet address.
    pub destination: Address,
    /// TON amount forwarded to the destination wallet.
    pub forward_ton_amount: u128,
    /// Internal transfer payload executed by the destination wallet.
    pub internal_transfer: JettonInternalTransfer,
}

impl JettonStandardMint {
    /// Creates a standard mint body.
    pub fn new(
        destination: Address,
        forward_ton_amount: u128,
        internal_transfer: JettonInternalTransfer,
    ) -> Self {
        Self {
            query_id: 0,
            destination,
            forward_ton_amount,
            internal_transfer,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonStandardMint {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(21)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, Some(&self.destination))?;
        builder.store_coins(self.forward_ton_amount)?;
        builder.store_ref(self.internal_transfer.to_cell()?)?;
        Ok(())
    }
}

/// Stablecoin `change_admin#6501f354` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonChangeAdmin {
    /// Query identifier.
    pub query_id: u64,
    /// New admin address, or none to drop admin rights.
    pub admin_address: Option<Address>,
}

impl JettonChangeAdmin {
    /// Creates a change-admin body.
    pub fn new(admin_address: Option<Address>) -> Self {
        Self {
            query_id: 0,
            admin_address,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonChangeAdmin {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x6501_f354)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, self.admin_address.as_ref())?;
        Ok(())
    }
}

/// Standard `change_admin#00000003` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonStandardChangeAdmin {
    /// Query identifier.
    pub query_id: u64,
    /// New admin address, or none to drop admin rights.
    pub admin_address: Option<Address>,
}

impl JettonStandardChangeAdmin {
    /// Creates a standard change-admin body.
    pub fn new(admin_address: Option<Address>) -> Self {
        Self {
            query_id: 0,
            admin_address,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonStandardChangeAdmin {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(3)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, self.admin_address.as_ref())?;
        Ok(())
    }
}

/// Stablecoin `change_metadata#cb862902` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonChangeContent {
    /// Query identifier.
    pub query_id: u64,
    /// New off-chain metadata.
    pub content: Tep64Content,
}

impl JettonChangeContent {
    /// Creates a stablecoin change-content body.
    pub fn new(content: Tep64Content) -> Self {
        Self {
            query_id: 0,
            content,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonChangeContent {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        let uri =
            self.content
                .off_chain_uri()
                .ok_or_else(|| crate::tlb::TlbError::CustomSchema {
                    schema: "JettonChangeContent",
                    message: "stablecoin metadata must be off-chain".to_string(),
                })?;
        builder.store_u32(0xcb86_2902)?;
        builder.store_u64(self.query_id)?;
        builder.store_snake_bytes(uri)?;
        Ok(())
    }
}

/// Standard `change_content#00000004` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonStandardChangeContent {
    /// Query identifier.
    pub query_id: u64,
    /// New on-chain or off-chain metadata.
    pub content: Tep64Content,
}

impl JettonStandardChangeContent {
    /// Creates a standard change-content body.
    pub fn new(content: Tep64Content) -> Self {
        Self {
            query_id: 0,
            content,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonStandardChangeContent {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        let content =
            self.content
                .to_cell()
                .map_err(|error| crate::tlb::TlbError::CustomSchema {
                    schema: "JettonStandardChangeContent",
                    message: error.to_string(),
                })?;
        builder.store_u32(4)?;
        builder.store_u64(self.query_id)?;
        builder.store_ref(content)?;
        Ok(())
    }
}

/// `top_up#d372158c` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonTopUp {
    /// Query identifier.
    pub query_id: u64,
}

impl JettonTopUp {
    /// Creates a top-up body.
    pub fn new() -> Self {
        Self { query_id: 0 }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl Default for JettonTopUp {
    fn default() -> Self {
        Self::new()
    }
}

impl TlbSerialize for JettonTopUp {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0xd372_158c)?;
        builder.store_u64(self.query_id)?;
        Ok(())
    }
}

/// `upgrade#2508d66a` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonUpgrade {
    /// Query identifier.
    pub query_id: u64,
    /// New contract code.
    pub code: Arc<Cell>,
    /// New contract data.
    pub data: Arc<Cell>,
}

impl JettonUpgrade {
    /// Creates an upgrade body.
    pub fn new(code: Arc<Cell>, data: Arc<Cell>) -> Self {
        Self {
            query_id: 0,
            code,
            data,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for JettonUpgrade {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x2508_d66a)?;
        builder.store_u64(self.query_id)?;
        builder.store_ref(self.data.clone())?;
        builder.store_ref(self.code.clone())?;
        Ok(())
    }
}
