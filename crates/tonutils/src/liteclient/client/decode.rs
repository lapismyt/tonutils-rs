use crate::{
    liteclient::{
        boc::{DecodedConfigInfo, LiteBocError, decode_optional_boc, decode_optional_config},
        tl::ConfigInfo,
        types::LiteError,
    },
    tvm::TvmError,
};

#[derive(Debug, thiserror::Error)]
pub(super) enum LiteDecodeError {
    #[error(transparent)]
    Boc(#[from] LiteBocError),
    #[error(transparent)]
    Tvm(#[from] TvmError),
}

impl From<LiteDecodeError> for LiteError {
    fn from(error: LiteDecodeError) -> Self {
        match error {
            LiteDecodeError::Boc(error) => Self::Boc(error),
            LiteDecodeError::Tvm(error) => Self::Decode(error),
        }
    }
}

pub(super) fn decode_config_info(raw: ConfigInfo) -> Result<DecodedConfigInfo, LiteDecodeError> {
    let state_proof = decode_optional_boc(&raw.state_proof)?;
    let config_proof = decode_optional_config(&raw.config_proof)?;
    Ok(DecodedConfigInfo {
        raw,
        state_proof,
        config_proof,
    })
}

pub(super) fn decode_error(error: impl Into<LiteDecodeError>) -> LiteError {
    error.into().into()
}
