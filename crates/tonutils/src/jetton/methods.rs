//! Typed jetton get-method wrappers.

use std::sync::Arc;

use num_bigint::BigUint;

use crate::{
    contracts::{
        ContractDefinition, ContractError, ensure_consumed, next_entry as next,
        run_get_method as run,
    },
    metadata::{Tep64Content, parse_tep64_content},
    provider::Provider,
    tvm::{Address, Cell},
};

use super::{
    master::{JettonMasterStablecoin, JettonMasterStablecoinV2, JettonMasterStandard},
    wallet::{JettonWalletStablecoin, JettonWalletStablecoinV2, JettonWalletStandard},
};

/// Decoded `get_jetton_data` result.
#[derive(Debug, Clone)]
pub struct JettonMasterInfo {
    /// Total minted supply in base units.
    pub total_supply: BigUint,
    /// Whether the master still allows minting.
    pub mintable: bool,
    /// Admin address, if any.
    pub admin_address: Option<Address>,
    /// Jetton metadata.
    pub content: Tep64Content,
    /// Wallet code published by the master.
    pub jetton_wallet_code: Arc<Cell>,
}

/// Decoded `get_wallet_data` result.
#[derive(Debug, Clone)]
pub struct JettonWalletInfo {
    /// Current jetton balance in base units.
    pub balance: BigUint,
    /// Wallet owner address.
    pub owner_address: Address,
    /// Jetton master address.
    pub jetton_master_address: Address,
    /// Wallet code, when the edition returns it.
    pub jetton_wallet_code: Option<Arc<Cell>>,
}

/// Runs `get_jetton_data` on a master.
pub async fn get_jetton_data<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<JettonMasterInfo, ContractError> {
    let entries = run(provider, address, "get_jetton_data", ()).await?;
    let mut entries = entries.into_iter();
    let total_supply = next(&mut entries)?;
    let mintable = next(&mut entries)?;
    let admin_address = next(&mut entries)?;
    let content_cell: Arc<Cell> = next(&mut entries)?;
    let jetton_wallet_code = next(&mut entries)?;
    ensure_consumed(&entries)?;
    let content =
        parse_tep64_content(content_cell).map_err(|error| ContractError::Data(Box::new(error)))?;
    Ok(JettonMasterInfo {
        total_supply,
        mintable,
        admin_address,
        content,
        jetton_wallet_code,
    })
}

/// Runs `get_wallet_address` on a master.
pub async fn get_wallet_address<P: Provider + ?Sized>(
    provider: &mut P,
    master: Address,
    owner: &Address,
) -> Result<Address, ContractError> {
    let entries = run(provider, master, "get_wallet_address", owner.clone()).await?;
    let mut entries = entries.into_iter();
    let address = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(address)
}

/// Runs `get_next_admin_address` on a stablecoin master.
pub async fn get_next_admin_address<P: Provider + ?Sized>(
    provider: &mut P,
    master: Address,
) -> Result<Option<Address>, ContractError> {
    let entries = run(provider, master, "get_next_admin_address", ()).await?;
    let mut entries = entries.into_iter();
    let address = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(address)
}

/// Runs `get_wallet_data` on a jetton wallet.
pub async fn get_wallet_data<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<JettonWalletInfo, ContractError> {
    let entries = run(provider, address, "get_wallet_data", ()).await?;
    let mut entries = entries.into_iter();
    let balance = next(&mut entries)?;
    let owner_address = next(&mut entries)?;
    let jetton_master_address = next(&mut entries)?;
    let jetton_wallet_code = if entries.len() != 0 {
        Some(next(&mut entries)?)
    } else {
        None
    };
    ensure_consumed(&entries)?;
    Ok(JettonWalletInfo {
        balance,
        owner_address,
        jetton_master_address,
        jetton_wallet_code,
    })
}

/// Runs `get_status` on a stablecoin jetton wallet.
pub async fn get_status<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<u8, ContractError> {
    let entries = run(provider, address, "get_status", ()).await?;
    let mut entries = entries.into_iter();
    let status = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(status)
}

impl JettonMasterStandard {
    /// Runs `get_jetton_data`.
    pub async fn get_jetton_data<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<JettonMasterInfo, ContractError> {
        get_jetton_data(provider, self.address()?).await
    }

    /// Runs `get_wallet_address`.
    pub async fn get_wallet_address<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
        owner: &Address,
    ) -> Result<Address, ContractError> {
        get_wallet_address(provider, self.address()?, owner).await
    }
}

impl JettonMasterStablecoin {
    /// Runs `get_jetton_data`.
    pub async fn get_jetton_data<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<JettonMasterInfo, ContractError> {
        get_jetton_data(provider, self.address()?).await
    }

    /// Runs `get_wallet_address`.
    pub async fn get_wallet_address<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
        owner: &Address,
    ) -> Result<Address, ContractError> {
        get_wallet_address(provider, self.address()?, owner).await
    }

    /// Runs `get_next_admin_address`.
    pub async fn get_next_admin_address<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<Option<Address>, ContractError> {
        get_next_admin_address(provider, self.address()?).await
    }
}

impl JettonMasterStablecoinV2 {
    /// Runs `get_jetton_data`.
    pub async fn get_jetton_data<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<JettonMasterInfo, ContractError> {
        get_jetton_data(provider, self.address()?).await
    }

    /// Runs `get_wallet_address`.
    pub async fn get_wallet_address<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
        owner: &Address,
    ) -> Result<Address, ContractError> {
        get_wallet_address(provider, self.address()?, owner).await
    }

    /// Runs `get_next_admin_address`.
    pub async fn get_next_admin_address<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<Option<Address>, ContractError> {
        get_next_admin_address(provider, self.address()?).await
    }
}

impl JettonWalletStandard {
    /// Runs `get_wallet_data`.
    pub async fn get_wallet_data<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<JettonWalletInfo, ContractError> {
        get_wallet_data(provider, self.address()?).await
    }
}

impl JettonWalletStablecoin {
    /// Runs `get_wallet_data`.
    pub async fn get_wallet_data<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<JettonWalletInfo, ContractError> {
        get_wallet_data(provider, self.address()?).await
    }

    /// Runs `get_status`.
    pub async fn get_status<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u8, ContractError> {
        get_status(provider, self.address()?).await
    }
}

impl JettonWalletStablecoinV2 {
    /// Runs `get_wallet_data`.
    pub async fn get_wallet_data<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<JettonWalletInfo, ContractError> {
        get_wallet_data(provider, self.address()?).await
    }
}
