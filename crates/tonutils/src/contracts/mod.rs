//! Smart-contract wrappers and get-method helpers.
//!
//! This module sits between [`crate::provider`] and protocol-specific helpers
//! such as [`crate::wallet`], [`crate::jetton`], and [`crate::nft`]. It does not
//! broadcast messages by itself; callers construct and submit messages through
//! an appropriate provider.
//!
//! Contract reads need a live provider, while address and message construction
//! can be performed offline. Returned data is only as trustworthy as the
//! provider and proof-validation policy selected by the application.
//!
//! [`ContractDefinition`] and [`ContractSpec`] provide the shared plumbing for
//! implementing contract wrappers: embedded default code, typed data, offline
//! `StateInit`/address derivation, and deploy requests.

pub mod codes;
mod definition;
mod get;
mod provider;
mod stack;

pub use crate::liteclient::{
    DecodedRunMethodResult, RUN_METHOD_MODE_RETURN_RESULT, RunMethodResultExt,
};
pub use codes::{ContractCodeError, ContractVersion};
pub use definition::{
    ContractData, ContractDefinition, ContractSpec, DeployRequest, ForwardPayload,
    build_state_init, internal_message, load_address_field, store_address_field,
};
pub use get::{ensure_consumed, next_entry, run_get_method};
pub use provider::{
    Contract, ContractBlueprint, ContractBuildError, ContractError, address_from_state_init,
};
pub use stack::{
    FromTvmStack, FromTvmStackEntry, ToTvmStack, ToTvmStackEntry, TvmStackConversionError,
};
