fn decode_config_info(raw: ConfigInfo) -> Result<DecodedConfigInfo> {
    let state_proof = decode_optional_boc(&raw.state_proof).map_err(decode_error)?;
    let config_proof = decode_optional_config(&raw.config_proof).map_err(decode_error)?;
    Ok(DecodedConfigInfo {
        raw,
        state_proof,
        config_proof,
    })
}

fn decode_error(error: anyhow::Error) -> LiteError {
    LiteError::TlError(crate::tl::TlError::ParseError(error.to_string()))
}
