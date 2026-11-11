//! Smart-contract state reads and get-method helpers.
//!
//! This module sits between [`crate::provider`] and protocol-specific helpers
//! such as [`crate::wallet`]. It does not broadcast messages by itself; callers
//! construct and submit messages through an appropriate provider.
//!
//! Contract reads need a live provider, while address and message construction
//! can be performed offline. Returned data is only as trustworthy as the
//! provider and proof-validation policy selected by the application.

mod provider;
mod stack;

pub use crate::liteclient::{
    DecodedRunMethodResult, RUN_METHOD_MODE_RETURN_RESULT, RunMethodResultExt,
};
pub use provider::{
    Contract, ContractBlueprint, ContractBuildError, ContractError, address_from_state_init,
};
