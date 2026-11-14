//! Jetton contract data models (TEP-74).
//!
//! Each edition has its own storage layout. Values are stored as TL-B so the
//! same types can be used for offline address calculation, deployment, and
//! decoding of on-chain account data.

use std::sync::Arc;

use crate::{
    contracts::{load_address_field, store_address_field},
    metadata::{Tep64Content, parse_tep64_content, parse_tep64_offchain},
    tlb::{TlbDeserialize, TlbError, TlbResult, TlbSerialize},
    tvm::{Address, Builder, Cell, Slice},
};

fn content_cell(content: &Tep64Content, with_prefix: bool) -> TlbResult<Arc<Cell>> {
    content
        .to_cell_with_prefix(with_prefix)
        .map_err(|error| TlbError::CustomSchema {
            schema: "Tep64Content",
            message: error.to_string(),
        })
}

fn parse_content(cell: Arc<Cell>, with_prefix: bool) -> TlbResult<Tep64Content> {
    let parsed = if with_prefix {
        parse_tep64_content(cell)
    } else {
        parse_tep64_offchain(cell)
    };
    parsed.map_err(|error| TlbError::CustomSchema {
        schema: "Tep64Content",
        message: error.to_string(),
    })
}

/// Standard jetton master storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMasterStandardData {
    /// Total minted supply in base units.
    pub total_supply: u128,
    /// Admin address with minting rights, if any.
    pub admin_address: Option<Address>,
    /// Jetton metadata, on-chain or off-chain.
    pub content: Tep64Content,
    /// Code cell used by jetton wallets of this master.
    pub jetton_wallet_code: Arc<Cell>,
}

impl TlbSerialize for JettonMasterStandardData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_coins(self.total_supply)?;
        store_address_field(builder, self.admin_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, true)?)?;
        builder.store_ref(self.jetton_wallet_code.clone())?;
        Ok(())
    }
}

impl TlbDeserialize for JettonMasterStandardData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            total_supply: slice.load_coins()?,
            admin_address: load_address_field(slice)?,
            content: parse_content(slice.load_reference()?, true)?,
            jetton_wallet_code: slice.load_reference()?,
        })
    }
}

/// Stablecoin jetton master storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMasterStablecoinData {
    /// Total minted supply in base units.
    pub total_supply: u128,
    /// Current admin address.
    pub admin_address: Option<Address>,
    /// Pending next admin address.
    pub next_admin_address: Option<Address>,
    /// Code cell used by jetton wallets of this master.
    pub jetton_wallet_code: Arc<Cell>,
    /// Off-chain metadata URI stored as a bare snake string.
    pub content: Tep64Content,
}

impl TlbSerialize for JettonMasterStablecoinData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_coins(self.total_supply)?;
        store_address_field(builder, self.admin_address.as_ref())?;
        store_address_field(builder, self.next_admin_address.as_ref())?;
        builder.store_ref(self.jetton_wallet_code.clone())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        Ok(())
    }
}

impl TlbDeserialize for JettonMasterStablecoinData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            total_supply: slice.load_coins()?,
            admin_address: load_address_field(slice)?,
            next_admin_address: load_address_field(slice)?,
            jetton_wallet_code: slice.load_reference()?,
            content: parse_content(slice.load_reference()?, false)?,
        })
    }
}

/// Standard jetton wallet storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonWalletStandardData {
    /// Current jetton balance in base units.
    pub balance: u128,
    /// Wallet owner address.
    pub owner_address: Option<Address>,
    /// Jetton master address.
    pub jetton_master_address: Option<Address>,
    /// Code cell used by this wallet.
    pub jetton_wallet_code: Arc<Cell>,
}

impl TlbSerialize for JettonWalletStandardData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_coins(self.balance)?;
        store_address_field(builder, self.owner_address.as_ref())?;
        store_address_field(builder, self.jetton_master_address.as_ref())?;
        builder.store_ref(self.jetton_wallet_code.clone())?;
        Ok(())
    }
}

impl TlbDeserialize for JettonWalletStandardData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            balance: slice.load_coins()?,
            owner_address: load_address_field(slice)?,
            jetton_master_address: load_address_field(slice)?,
            jetton_wallet_code: slice.load_reference()?,
        })
    }
}

/// Stablecoin jetton wallet storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonWalletStablecoinData {
    /// Four-bit wallet status flags.
    pub status: u8,
    /// Current jetton balance in base units.
    pub balance: u128,
    /// Wallet owner address.
    pub owner_address: Option<Address>,
    /// Jetton master address.
    pub jetton_master_address: Option<Address>,
}

impl TlbSerialize for JettonWalletStablecoinData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_uint_custom(self.status, 4)?;
        builder.store_coins(self.balance)?;
        store_address_field(builder, self.owner_address.as_ref())?;
        store_address_field(builder, self.jetton_master_address.as_ref())?;
        Ok(())
    }
}

impl TlbDeserialize for JettonWalletStablecoinData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            status: slice.load_uint_custom(4)?,
            balance: slice.load_coins()?,
            owner_address: load_address_field(slice)?,
            jetton_master_address: load_address_field(slice)?,
        })
    }
}

/// Stablecoin V2 jetton wallet storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonWalletStablecoinV2Data {
    /// Current jetton balance in base units.
    pub balance: u128,
    /// Wallet owner address.
    pub owner_address: Option<Address>,
    /// Jetton master address.
    pub jetton_master_address: Option<Address>,
}

impl TlbSerialize for JettonWalletStablecoinV2Data {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_coins(self.balance)?;
        store_address_field(builder, self.owner_address.as_ref())?;
        store_address_field(builder, self.jetton_master_address.as_ref())?;
        Ok(())
    }
}

impl TlbDeserialize for JettonWalletStablecoinV2Data {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            balance: slice.load_coins()?,
            owner_address: load_address_field(slice)?,
            jetton_master_address: load_address_field(slice)?,
        })
    }
}
