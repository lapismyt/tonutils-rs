//! Helpers for decoding `liteServer.runMethodResult` payloads.

use crate::{
    liteclient::tl::RunMethodResult,
    tvm::{TvmResult, TvmStack},
};

/// LiteAPI `runSmcMethod` mode bit that requests the result stack BoC.
pub const RUN_METHOD_MODE_RETURN_RESULT: u32 = 1 << 2;

/// Lossless decoded view of optional `liteServer.runMethodResult.result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedRunMethodResult {
    /// The liteserver did not return a result BoC.
    Missing,
    /// The result BoC decoded into a TVM stack.
    Decoded(TvmStack),
    /// The result BoC was present but could not be decoded.
    Undecodable { raw: Vec<u8>, error: String },
}

/// Convenience accessors for raw LiteAPI get-method results.
pub trait RunMethodResultExt {
    /// Returns the raw result stack BoC when present.
    fn raw_result_boc(&self) -> Option<&[u8]>;
    /// Decodes the result stack BoC into a [`TvmStack`].
    fn decode_result_stack(&self) -> TvmResult<Option<TvmStack>>;
    /// Returns a lossless decoded view that preserves undecodable bytes.
    fn result_stack_lossless(&self) -> DecodedRunMethodResult;
}

impl RunMethodResultExt for RunMethodResult {
    fn raw_result_boc(&self) -> Option<&[u8]> {
        self.result.as_deref()
    }

    fn decode_result_stack(&self) -> TvmResult<Option<TvmStack>> {
        self.result.as_deref().map(TvmStack::from_boc).transpose()
    }

    fn result_stack_lossless(&self) -> DecodedRunMethodResult {
        match self.result.as_deref() {
            None => DecodedRunMethodResult::Missing,
            Some(raw) => match TvmStack::from_boc(raw) {
                Ok(stack) => DecodedRunMethodResult::Decoded(stack),
                Err(error) => DecodedRunMethodResult::Undecodable {
                    raw: raw.to_vec(),
                    error: error.to_string(),
                },
            },
        }
    }
}
