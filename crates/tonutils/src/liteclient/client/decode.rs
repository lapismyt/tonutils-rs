use crate::{
    liteclient::{
        boc::{DecodedConfigInfo, decode_optional_boc, decode_optional_config},
        types::LiteError,
    },
    tl::{TlError, response::ConfigInfo},
};

// TODO: don't use anyhow in libraries
pub(super) fn decode_config_info(raw: ConfigInfo) -> anyhow::Result<DecodedConfigInfo> {
    let state_proof = decode_optional_boc(&raw.state_proof).map_err(decode_error)?;
    let config_proof = decode_optional_config(&raw.config_proof).map_err(decode_error)?;
    Ok(DecodedConfigInfo {
        raw,
        state_proof,
        config_proof,
    })
}

pub(super) fn decode_error(error: anyhow::Error) -> LiteError {
    LiteError::TlError(TlError::ParseError(error.to_string()))
}
