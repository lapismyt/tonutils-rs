//! Wallet contract definitions and get-methods.
//!
//! Wallets are contracts too: they have embedded code, typed storage data, an
//! offline address, and get-methods. These impls let wallets interoperate with
//! the rest of [`crate::contracts`] and with deploy helpers.

use std::sync::Arc;

use crate::{
    contracts::{
        ContractCodeError, ContractDefinition, ContractError, ensure_consumed, next_entry as next,
        run_get_method as run,
    },
    provider::Provider,
    tvm::{Address, Cell},
};

use super::message::{WalletV4R2, WalletV5R1};
use super::mnemonic::{
    WalletError, WalletV4R2Data, WalletV5R1Data, wallet_v4r2_code, wallet_v5r1_code,
};

/// Decoded wallet get-method state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletState {
    /// Current sequence number.
    pub seqno: u32,
    /// Raw wallet id.
    pub wallet_id: u32,
    /// Ed25519 public key bytes.
    pub public_key: [u8; 32],
}

/// Runs `seqno` on a wallet.
pub async fn get_seqno<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<u32, ContractError> {
    let entries = run(provider, address, "seqno", ()).await?;
    let mut entries = entries.into_iter();
    let seqno = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(seqno)
}

/// Runs `get_subwallet_id` on a wallet.
pub async fn get_subwallet_id<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<u32, ContractError> {
    let entries = run(provider, address, "get_subwallet_id", ()).await?;
    let mut entries = entries.into_iter();
    let wallet_id = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(wallet_id)
}

/// Runs `get_public_key` on a wallet.
pub async fn get_public_key<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<[u8; 32], ContractError> {
    let entries = run(provider, address, "get_public_key", ()).await?;
    let mut entries = entries.into_iter();
    let key: num_bigint::BigUint = next(&mut entries)?;
    ensure_consumed(&entries)?;
    let digits = key.to_bytes_be();
    if digits.len() > 32 {
        return Err(ContractError::Data(Box::new(KeyLengthError {
            len: digits.len(),
        })));
    }
    let mut public_key = [0u8; 32];
    public_key[32 - digits.len()..].copy_from_slice(&digits);
    Ok(public_key)
}

/// Runs `get_is_signature_auth_allowed` on a Wallet V5R1.
pub async fn get_is_signature_auth_allowed<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<bool, ContractError> {
    let entries = run(provider, address, "get_is_signature_auth_allowed", ()).await?;
    let mut entries = entries.into_iter();
    let allowed = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(allowed)
}

/// Reads the common wallet state in one place.
pub async fn get_wallet_state<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<WalletState, ContractError> {
    let seqno = get_seqno(provider, address.clone()).await?;
    let wallet_id = get_subwallet_id(provider, address.clone()).await?;
    let public_key = get_public_key(provider, address).await?;
    Ok(WalletState {
        seqno,
        wallet_id,
        public_key,
    })
}

impl WalletV4R2 {
    /// Runs `seqno`.
    pub async fn get_seqno<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u32, ContractError> {
        get_seqno(provider, contract_address(self)?).await
    }

    /// Runs `get_subwallet_id`.
    pub async fn get_subwallet_id<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u32, ContractError> {
        get_subwallet_id(provider, contract_address(self)?).await
    }

    /// Runs `get_public_key`.
    pub async fn get_public_key<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<[u8; 32], ContractError> {
        get_public_key(provider, contract_address(self)?).await
    }

    /// Reads the common wallet state.
    pub async fn get_wallet_state<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<WalletState, ContractError> {
        get_wallet_state(provider, contract_address(self)?).await
    }
}

impl WalletV5R1 {
    /// Runs `seqno`.
    pub async fn get_seqno<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u32, ContractError> {
        get_seqno(provider, contract_address(self)?).await
    }

    /// Runs `get_subwallet_id`.
    pub async fn get_subwallet_id<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u32, ContractError> {
        get_subwallet_id(provider, contract_address(self)?).await
    }

    /// Runs `get_public_key`.
    pub async fn get_public_key<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<[u8; 32], ContractError> {
        get_public_key(provider, contract_address(self)?).await
    }

    /// Runs `get_is_signature_auth_allowed`.
    pub async fn get_is_signature_auth_allowed<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<bool, ContractError> {
        get_is_signature_auth_allowed(provider, contract_address(self)?).await
    }

    /// Reads the common wallet state.
    pub async fn get_wallet_state<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<WalletState, ContractError> {
        get_wallet_state(provider, contract_address(self)?).await
    }
}

impl ContractDefinition for WalletV4R2 {
    type Data = WalletV4R2Data;

    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        wallet_v4r2_code().map_err(|error| wallet_code_error(&error))
    }

    fn code(&self) -> &Arc<Cell> {
        &self.code
    }

    fn data(&self) -> Self::Data {
        Self::data(self)
    }

    fn workchain(&self) -> i8 {
        self.workchain()
    }
}

impl ContractDefinition for WalletV5R1 {
    type Data = WalletV5R1Data;

    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        wallet_v5r1_code().map_err(|error| wallet_code_error(&error))
    }

    fn code(&self) -> &Arc<Cell> {
        &self.code
    }

    fn data(&self) -> Self::Data {
        Self::data(self)
    }

    fn workchain(&self) -> i8 {
        self.workchain()
    }
}

fn contract_address<T: ContractDefinition>(wallet: &T) -> Result<Address, ContractError> {
    use crate::contracts::ContractBuildError;
    let state_init = wallet.state_init()?;
    crate::contracts::address_from_state_init(wallet.workchain(), &state_init)
        .map_err(ContractBuildError::StateInitSerialization)
        .map_err(ContractError::Build)
}

fn wallet_code_error(error: &WalletError) -> ContractCodeError {
    ContractCodeError::InvalidBoc {
        version: "Wallet",
        message: error.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("public key has {len} bytes, expected at most 32")]
struct KeyLengthError {
    len: usize,
}
