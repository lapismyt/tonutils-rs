//! Jetton wallet wrappers.

use std::sync::Arc;

use crate::{
    contracts::{
        ContractBuildError, ContractCodeError, ContractDefinition, ContractSpec, ContractVersion,
    },
    tvm::{Address, Cell},
};

use super::data::{
    JettonWalletStablecoinData, JettonWalletStablecoinV2Data, JettonWalletStandardData,
};
/// Standard jetton wallet (TEP-74).
#[derive(Debug, Clone)]
pub struct JettonWalletStandard {
    spec: ContractSpec<JettonWalletStandardData>,
}

impl JettonWalletStandard {
    /// Creates a wallet with the embedded default code.
    pub fn new(
        owner_address: Address,
        jetton_master_address: Address,
        balance: u128,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::JettonWalletStandard,
                JettonWalletStandardData {
                    balance,
                    owner_address: Some(owner_address),
                    jetton_master_address: Some(jetton_master_address),
                    jetton_wallet_code: ContractVersion::JettonWalletStandard.default_code()?,
                },
            )?,
        })
    }

    /// Creates a wallet with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        owner_address: Address,
        jetton_master_address: Address,
        balance: u128,
    ) -> Self {
        Self::from_parts(
            code.clone(),
            JettonWalletStandardData {
                balance,
                owner_address: Some(owner_address),
                jetton_master_address: Some(jetton_master_address),
                jetton_wallet_code: code,
            },
        )
    }

    /// Creates a wallet from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: JettonWalletStandardData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a wallet from decoded data and the embedded default code.
    pub fn from_data(data: JettonWalletStandardData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::JettonWalletStandard, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Current jetton balance.
    pub fn balance(&self) -> u128 {
        self.spec.data.balance
    }

    /// Wallet owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Jetton master address.
    pub fn jetton_master_address(&self) -> Option<&Address> {
        self.spec.data.jetton_master_address.as_ref()
    }
}

impl ContractDefinition for JettonWalletStandard {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::JettonWalletStandard.default_code()
    }
    type Data = JettonWalletStandardData;

    fn code(&self) -> &Arc<Cell> {
        &self.spec.code
    }

    fn data(&self) -> Self::Data {
        self.spec.data.clone()
    }

    fn workchain(&self) -> i8 {
        self.spec.workchain
    }
}

/// Stablecoin jetton wallet.
#[derive(Debug, Clone)]
pub struct JettonWalletStablecoin {
    spec: ContractSpec<JettonWalletStablecoinData>,
}

impl JettonWalletStablecoin {
    /// Creates a wallet with the embedded default code.
    pub fn new(
        owner_address: Address,
        jetton_master_address: Address,
        status: u8,
        balance: u128,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::JettonWalletStablecoin,
                JettonWalletStablecoinData {
                    status,
                    balance,
                    owner_address: Some(owner_address),
                    jetton_master_address: Some(jetton_master_address),
                },
            )?,
        })
    }

    /// Creates a wallet with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        owner_address: Address,
        jetton_master_address: Address,
        status: u8,
        balance: u128,
    ) -> Self {
        Self {
            spec: ContractSpec::with_code(
                code,
                JettonWalletStablecoinData {
                    status,
                    balance,
                    owner_address: Some(owner_address),
                    jetton_master_address: Some(jetton_master_address),
                },
            ),
        }
    }

    /// Creates a wallet from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: JettonWalletStablecoinData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a wallet from decoded data and the embedded default code.
    pub fn from_data(data: JettonWalletStablecoinData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::JettonWalletStablecoin, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Four-bit wallet status.
    pub fn status(&self) -> u8 {
        self.spec.data.status
    }

    /// Current jetton balance.
    pub fn balance(&self) -> u128 {
        self.spec.data.balance
    }

    /// Wallet owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Jetton master address.
    pub fn jetton_master_address(&self) -> Option<&Address> {
        self.spec.data.jetton_master_address.as_ref()
    }
}

impl ContractDefinition for JettonWalletStablecoin {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::JettonWalletStablecoin.default_code()
    }
    type Data = JettonWalletStablecoinData;

    fn code(&self) -> &Arc<Cell> {
        &self.spec.code
    }

    fn data(&self) -> Self::Data {
        self.spec.data.clone()
    }

    fn workchain(&self) -> i8 {
        self.spec.workchain
    }
}

/// Sharded stablecoin jetton wallet (V2).
#[derive(Debug, Clone)]
pub struct JettonWalletStablecoinV2 {
    spec: ContractSpec<JettonWalletStablecoinV2Data>,
}

impl JettonWalletStablecoinV2 {
    /// Creates a wallet with the embedded default code.
    pub fn new(
        owner_address: Address,
        jetton_master_address: Address,
        balance: u128,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::JettonWalletStablecoinV2,
                JettonWalletStablecoinV2Data {
                    balance,
                    owner_address: Some(owner_address),
                    jetton_master_address: Some(jetton_master_address),
                },
            )?,
        })
    }

    /// Creates a wallet with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        owner_address: Address,
        jetton_master_address: Address,
        balance: u128,
    ) -> Self {
        Self {
            spec: ContractSpec::with_code(
                code,
                JettonWalletStablecoinV2Data {
                    balance,
                    owner_address: Some(owner_address),
                    jetton_master_address: Some(jetton_master_address),
                },
            ),
        }
    }

    /// Creates a wallet from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: JettonWalletStablecoinV2Data) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a wallet from decoded data and the embedded default code.
    pub fn from_data(data: JettonWalletStablecoinV2Data) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::JettonWalletStablecoinV2, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Current jetton balance.
    pub fn balance(&self) -> u128 {
        self.spec.data.balance
    }

    /// Wallet owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Jetton master address.
    pub fn jetton_master_address(&self) -> Option<&Address> {
        self.spec.data.jetton_master_address.as_ref()
    }

    /// Computes the sharded address for this wallet's owner and master.
    pub fn shard_address(&self) -> Result<Address, ContractBuildError> {
        let owner = self
            .spec
            .data
            .owner_address
            .as_ref()
            .ok_or(ContractBuildError::MissingField("owner_address"))?;
        let master = self
            .spec
            .data
            .jetton_master_address
            .as_ref()
            .ok_or(ContractBuildError::MissingField("jetton_master_address"))?;
        super::master::JettonMasterStablecoinV2::calculate_wallet_address(
            owner,
            master,
            &self.spec.code,
            self.spec.workchain,
        )
    }
}

impl ContractDefinition for JettonWalletStablecoinV2 {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::JettonWalletStablecoinV2.default_code()
    }
    type Data = JettonWalletStablecoinV2Data;

    fn code(&self) -> &Arc<Cell> {
        &self.spec.code
    }

    fn data(&self) -> Self::Data {
        self.spec.data.clone()
    }

    fn workchain(&self) -> i8 {
        self.spec.workchain
    }

    fn address(&self) -> Result<Address, ContractBuildError> {
        self.shard_address()
    }
}
