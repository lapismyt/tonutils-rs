//! Thanks to <https://github.com/tonstack/lite-client>

pub mod balancer;
pub mod boc;
pub mod client;
pub mod layers;
pub mod peer;
pub mod rate_limit;
mod response;
pub mod run_method;
pub mod server;
pub mod tl;
pub mod types;

#[cfg(test)]
mod tests;

pub use balancer::LiteBalancer;
pub use client::LiteClient;
pub use response::FromResponse;
pub use run_method::{DecodedRunMethodResult, RUN_METHOD_MODE_RETURN_RESULT, RunMethodResultExt};
