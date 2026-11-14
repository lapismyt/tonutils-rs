//! Jetton master wrappers.

use std::sync::Arc;

use crate::{
    contracts::{
        ContractCodeError, ContractDefinition, ContractSpec, ContractVersion, build_state_init,
    },
    metadata::Tep64Content,
    tlb::{StateInit, TlbSerialize},
    tvm::{Address, Cell},
};

use super::data::{JettonMasterStablecoinData, JettonMasterStandardData, JettonWalletStandardData};

/// Standard jetton master (TEP-74).
#[derive(Debug, Clone)]
pub struct JettonMasterStandard {
    spec: ContractSpec<JettonMasterStandardData>,
}

impl JettonMasterStandard {
    /// Creates a master with the embedded default code and wallet code.
    pub fn new(
        admin_address: Option<Address>,
        content: Tep64Content,
    ) -> Result<Self, ContractCodeError> {
        let wallet_code = ContractVersion::JettonWalletStandard.default_code()?;
        Self::with_wallet_code(admin_address, content, wallet_code)
    }

    /// Creates a master with the embedded default master code and custom wallet code.
    pub fn with_wallet_code(
        admin_address: Option<Address>,
        content: Tep64Content,
        jetton_wallet_code: Arc<Cell>,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::JettonMasterStandard,
                JettonMasterStandardData {
                    total_supply: 0,
                    admin_address,
                    content,
                    jetton_wallet_code,
                },
            )?,
        })
    }

    /// Creates a master with explicit master code and wallet code.
    pub fn with_code(
        code: Arc<Cell>,
        admin_address: Option<Address>,
        content: Tep64Content,
        jetton_wallet_code: Arc<Cell>,
    ) -> Self {
        Self::from_parts(
            code,
            JettonMasterStandardData {
                total_supply: 0,
                admin_address,
                content,
                jetton_wallet_code,
            },
        )
    }

    /// Creates a master from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: JettonMasterStandardData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a master from data and the embedded default code.
    pub fn from_data(data: JettonMasterStandardData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::JettonMasterStandard, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Total minted supply.
    pub fn total_supply(&self) -> u128 {
        self.spec.data.total_supply
    }

    /// Admin address.
    pub fn admin_address(&self) -> Option<&Address> {
        self.spec.data.admin_address.as_ref()
    }

    /// Jetton metadata.
    pub fn content(&self) -> &Tep64Content {
        &self.spec.data.content
    }

    /// Wallet code published by this master.
    pub fn jetton_wallet_code(&self) -> &Arc<Cell> {
        &self.spec.data.jetton_wallet_code
    }

    /// Computes a jetton wallet address for `owner` using this master's code.
    pub fn wallet_address(
        &self,
        owner: &Address,
    ) -> Result<Address, crate::contracts::ContractBuildError> {
        let master = self.address()?;
        Self::calculate_wallet_address(
            owner,
            &master,
            &self.spec.data.jetton_wallet_code,
            self.spec.workchain,
        )
    }

    /// Computes a standard jetton wallet address offline.
    pub fn calculate_wallet_address(
        owner: &Address,
        master: &Address,
        wallet_code: &Arc<Cell>,
        workchain: i8,
    ) -> Result<Address, crate::contracts::ContractBuildError> {
        let data = JettonWalletStandardData {
            balance: 0,
            owner_address: Some(owner.clone()),
            jetton_master_address: Some(master.clone()),
            jetton_wallet_code: wallet_code.clone(),
        };
        let state_init = build_state_init(wallet_code, &data)?;
        crate::contracts::address_from_state_init(workchain, &state_init)
            .map_err(crate::contracts::ContractBuildError::StateInitSerialization)
    }
}

impl ContractDefinition for JettonMasterStandard {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::JettonMasterStandard.default_code()
    }
    type Data = JettonMasterStandardData;

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

/// Stablecoin jetton master.
#[derive(Debug, Clone)]
pub struct JettonMasterStablecoin {
    spec: ContractSpec<JettonMasterStablecoinData>,
}

impl JettonMasterStablecoin {
    /// Creates a master with the embedded default code and wallet code.
    pub fn new(
        admin_address: Option<Address>,
        next_admin_address: Option<Address>,
        content: Tep64Content,
    ) -> Result<Self, ContractCodeError> {
        let wallet_code = ContractVersion::JettonWalletStablecoin.default_code()?;
        Self::with_wallet_code(admin_address, next_admin_address, content, wallet_code)
    }

    /// Creates a master with a custom wallet code.
    pub fn with_wallet_code(
        admin_address: Option<Address>,
        next_admin_address: Option<Address>,
        content: Tep64Content,
        jetton_wallet_code: Arc<Cell>,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::JettonMasterStablecoin,
                JettonMasterStablecoinData {
                    total_supply: 0,
                    admin_address,
                    next_admin_address,
                    jetton_wallet_code,
                    content,
                },
            )?,
        })
    }

    /// Creates a master with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        admin_address: Option<Address>,
        next_admin_address: Option<Address>,
        content: Tep64Content,
        jetton_wallet_code: Arc<Cell>,
    ) -> Self {
        Self::from_parts(
            code,
            JettonMasterStablecoinData {
                total_supply: 0,
                admin_address,
                next_admin_address,
                jetton_wallet_code,
                content,
            },
        )
    }

    /// Creates a master from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: JettonMasterStablecoinData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a master from data and the embedded default code.
    pub fn from_data(data: JettonMasterStablecoinData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::JettonMasterStablecoin, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Total minted supply.
    pub fn total_supply(&self) -> u128 {
        self.spec.data.total_supply
    }

    /// Current admin address.
    pub fn admin_address(&self) -> Option<&Address> {
        self.spec.data.admin_address.as_ref()
    }

    /// Pending next admin address.
    pub fn next_admin_address(&self) -> Option<&Address> {
        self.spec.data.next_admin_address.as_ref()
    }

    /// Off-chain metadata.
    pub fn content(&self) -> &Tep64Content {
        &self.spec.data.content
    }

    /// Wallet code published by this master.
    pub fn jetton_wallet_code(&self) -> &Arc<Cell> {
        &self.spec.data.jetton_wallet_code
    }

    /// Computes a jetton wallet address for `owner`.
    pub fn wallet_address(
        &self,
        owner: &Address,
    ) -> Result<Address, crate::contracts::ContractBuildError> {
        let master = self.address()?;
        Self::calculate_wallet_address(
            owner,
            &master,
            &self.spec.data.jetton_wallet_code,
            self.spec.workchain,
        )
    }

    /// Computes a stablecoin jetton wallet address offline.
    pub fn calculate_wallet_address(
        owner: &Address,
        master: &Address,
        wallet_code: &Arc<Cell>,
        workchain: i8,
    ) -> Result<Address, crate::contracts::ContractBuildError> {
        let data = super::data::JettonWalletStablecoinData {
            status: 0,
            balance: 0,
            owner_address: Some(owner.clone()),
            jetton_master_address: Some(master.clone()),
        };
        let state_init = build_state_init(wallet_code, &data)?;
        crate::contracts::address_from_state_init(workchain, &state_init)
            .map_err(crate::contracts::ContractBuildError::StateInitSerialization)
    }
}

impl ContractDefinition for JettonMasterStablecoin {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::JettonMasterStablecoin.default_code()
    }
    type Data = JettonMasterStablecoinData;

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

/// Sharded stablecoin jetton master (V2).
#[derive(Debug, Clone)]
pub struct JettonMasterStablecoinV2 {
    spec: ContractSpec<JettonMasterStablecoinData>,
}

impl JettonMasterStablecoinV2 {
    /// Number of address bits taken from the owner's hash.
    pub const SHARD_PREFIX_BITS: usize = 8;

    /// Creates a master with the embedded default code and wallet code.
    pub fn new(
        admin_address: Option<Address>,
        next_admin_address: Option<Address>,
        content: Tep64Content,
    ) -> Result<Self, ContractCodeError> {
        let wallet_code = ContractVersion::JettonWalletStablecoinV2.default_code()?;
        Self::with_wallet_code(admin_address, next_admin_address, content, wallet_code)
    }

    /// Creates a master with a custom wallet code.
    pub fn with_wallet_code(
        admin_address: Option<Address>,
        next_admin_address: Option<Address>,
        content: Tep64Content,
        jetton_wallet_code: Arc<Cell>,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::JettonMasterStablecoinV2,
                JettonMasterStablecoinData {
                    total_supply: 0,
                    admin_address,
                    next_admin_address,
                    jetton_wallet_code,
                    content,
                },
            )?,
        })
    }

    /// Creates a master with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        admin_address: Option<Address>,
        next_admin_address: Option<Address>,
        content: Tep64Content,
        jetton_wallet_code: Arc<Cell>,
    ) -> Self {
        Self::from_parts(
            code,
            JettonMasterStablecoinData {
                total_supply: 0,
                admin_address,
                next_admin_address,
                jetton_wallet_code,
                content,
            },
        )
    }

    /// Creates a master from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: JettonMasterStablecoinData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a master from data and the embedded default code.
    pub fn from_data(data: JettonMasterStablecoinData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::JettonMasterStablecoinV2, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Total minted supply.
    pub fn total_supply(&self) -> u128 {
        self.spec.data.total_supply
    }

    /// Current admin address.
    pub fn admin_address(&self) -> Option<&Address> {
        self.spec.data.admin_address.as_ref()
    }

    /// Pending next admin address.
    pub fn next_admin_address(&self) -> Option<&Address> {
        self.spec.data.next_admin_address.as_ref()
    }

    /// Off-chain metadata.
    pub fn content(&self) -> &Tep64Content {
        &self.spec.data.content
    }

    /// Wallet code published by this master.
    pub fn jetton_wallet_code(&self) -> &Arc<Cell> {
        &self.spec.data.jetton_wallet_code
    }

    /// Computes a sharded jetton wallet address for `owner`.
    pub fn wallet_address(
        &self,
        owner: &Address,
    ) -> Result<Address, crate::contracts::ContractBuildError> {
        let master = self.address()?;
        Self::calculate_wallet_address(
            owner,
            &master,
            &self.spec.data.jetton_wallet_code,
            self.spec.workchain,
        )
    }

    /// Computes a sharded stablecoin V2 jetton wallet address offline.
    ///
    /// The top eight bits of the state-init hash are replaced with the top eight
    /// bits of the owner's hash, matching the on-chain shard scheme.
    pub fn calculate_wallet_address(
        owner: &Address,
        master: &Address,
        wallet_code: &Arc<Cell>,
        workchain: i8,
    ) -> Result<Address, crate::contracts::ContractBuildError> {
        let data = super::data::JettonWalletStablecoinV2Data {
            balance: 0,
            owner_address: Some(owner.clone()),
            jetton_master_address: Some(master.clone()),
        };
        let data_cell = data
            .to_cell()
            .map_err(crate::contracts::ContractBuildError::DataSerialization)?;
        let state_init = StateInit {
            fixed_prefix_length: Some(8),
            code: Some(wallet_code.clone()),
            data: Some(data_cell),
            ..StateInit::empty()
        };
        let mut hash = state_init
            .to_cell()
            .map_err(crate::tlb::tlb_to_tvm_error)
            .map_err(crate::contracts::ContractBuildError::StateInitSerialization)?
            .hash();
        hash[0] = owner.hash_part[0];
        Ok(Address::new(workchain, hash))
    }
}

impl ContractDefinition for JettonMasterStablecoinV2 {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::JettonMasterStablecoinV2.default_code()
    }
    type Data = JettonMasterStablecoinData;

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
