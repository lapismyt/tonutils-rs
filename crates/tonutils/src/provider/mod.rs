//! Capability-based blockchain providers.
//!
//! The module exposes small traits instead of one monolithic provider:
//!
//! - [`GetMasterchainInfoExt`]
//! - [`GetAccountStateExt`]
//! - [`RunGetMethodExt`]
//! - [`SendExternalMessageExt`]
//! - [`GetTransactionsExt`]
//!
//! [`Provider`] is a convenience marker for implementations that support every
//! capability, but callers should depend only on the traits they use so the
//! compiler enforces feature availability.
//!
//! # Trust boundary
//!
//! HTTP providers (TonAPI, Toncenter v2, Toncenter v3) trust the remote service
//! and do not verify Merkle proofs. LiteClient and LiteBalancer adapters talk
//! directly to liteservers but also return responses without proof validation.
//! Applications that require cryptographic verification must validate proofs
//! explicitly or use the typed LiteClient proof helpers.

mod error;
mod http;
pub mod lite;
mod stack;
pub mod tonapi;
pub mod toncenter;
mod traits;
mod types;

pub use error::{ProviderError, ProviderResult};
pub use tonapi::{TonApi, TonApiBuilder};
pub use toncenter::{ToncenterV2, ToncenterV2Builder, ToncenterV3, ToncenterV3Builder};
pub use traits::{
    GetAccountStateExt, GetMasterchainInfoExt, GetTransactionsExt, Provider, RunGetMethodExt,
    SendExternalMessageExt,
};
pub use types::{
    AccountState, BlockId, MasterchainInfo, MethodRef, RunMethodResult, SendMessageResult,
    Transaction, TransactionList,
};
