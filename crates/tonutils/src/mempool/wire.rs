//! External-message BoC envelope validation and unwrapping.

use crate::{
    overlay::tl::TonNodeExternalMessageBroadcast,
    tlb::{CommonMsgInfo, Message, MsgAddressInt, TlbDeserialize},
    tvm::deserialize_boc,
};

use super::{MempoolConfig, MempoolError};

pub(crate) fn unwrap_external_message_payload(payload: Vec<u8>) -> Vec<u8> {
    match tl_proto::deserialize::<TonNodeExternalMessageBroadcast>(&payload) {
        Ok(broadcast) => broadcast.message.data,
        Err(_) => payload,
    }
}

pub(crate) fn decode_external(raw_boc: &[u8]) -> Result<Message, MempoolError> {
    let cell = deserialize_boc(raw_boc).map_err(|error| MempoolError::Decode(error.to_string()))?;
    TlbDeserialize::from_cell(cell).map_err(|error| MempoolError::Decode(error.to_string()))
}

pub(crate) fn destination(raw_boc: &[u8]) -> Result<Option<[u8; 32]>, MempoolError> {
    let message = decode_external(raw_boc)?;
    Ok(match message.info {
        CommonMsgInfo::ExternalIn { dest, .. } => match dest {
            MsgAddressInt::Std { address, .. } => Some(address.hash_part),
            MsgAddressInt::Var { .. } => None,
        },
        _ => None,
    })
}

pub(crate) fn check_envelope_header(
    raw_boc: &[u8],
    config: &MempoolConfig,
) -> Result<(), MempoolError> {
    if raw_boc.len() < 4 {
        return Err(MempoolError::InvalidEnvelope);
    }
    if raw_boc.len() > config.max_message_size {
        return Err(MempoolError::MessageTooLarge);
    }
    if config.require_boc_magic && raw_boc[..4] != [0xb5, 0xee, 0x9c, 0x72] {
        return Err(MempoolError::InvalidBoc);
    }
    Ok(())
}

pub(crate) fn validate_envelope(
    raw_boc: &[u8],
    config: &MempoolConfig,
) -> Result<Option<[u8; 32]>, MempoolError> {
    check_envelope_header(raw_boc, config)?;
    if !config.validate_message {
        return Ok(None);
    }
    let cell = deserialize_boc(raw_boc).map_err(|_| MempoolError::InvalidBoc)?;
    let message = Message::from_cell(cell).map_err(|_| MempoolError::InvalidMessage)?;
    match message.info {
        CommonMsgInfo::ExternalIn { dest, .. } => Ok(match dest {
            MsgAddressInt::Std { address, .. } => Some(address.hash_part),
            MsgAddressInt::Var { .. } => None,
        }),
        _ => Err(MempoolError::InvalidMessage),
    }
}
