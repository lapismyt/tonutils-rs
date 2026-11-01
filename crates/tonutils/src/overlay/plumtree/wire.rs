//! Plumtree identity, signing, and RaptorQ helpers.

use std::time::{SystemTime, UNIX_EPOCH};

use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};

use crate::{
    adnl::{AdnlKeyPair, AdnlPublicKey, tl::AdnlTlPublicKey as TlPublicKey},
    overlay::{
        OverlayError, OverlayId,
        broadcast::SIMPLE_BROADCAST_MAX,
        tl::{
            OverlayBroadcastPlumtreeFecId, OverlayBroadcastPlumtreeFecToSign,
            OverlayBroadcastPlumtreeSimpleToSign, OverlayCertificate, OverlayCertificateId,
            OverlayCertificateIdV2, OverlayQuery, OverlayRepairPlumtreePart,
        },
    },
    tl::TlInt256 as Int256,
};

use super::{PartInput, PlumtreeConfig, state::RepairKey};

/// Upstream `Overlays::max_fec_broadcast_size()`.
pub(crate) const MAX_PLUMTREE_BROADCAST_SIZE: usize = 16 << 20;
/// Upstream accepts payload timestamps within this window of local time.
pub(crate) const PLUMTREE_TIMESTAMP_WINDOW_SECONDS: f64 = 20.0;
pub(crate) const PLUMTREE_SIMPLE_TREE_INDEX: u32 = 0;
pub(crate) const PLUMTREE_FEC_TREE_OFFSET: u32 = 1;
/// `CertificateFlags::AllowFec` in upstream `overlay/overlays.h`.
const CERT_FLAG_ALLOW_FEC: u32 = 1;
/// `CertificateFlags::Trusted` in upstream `overlay/overlays.h`.
const CERT_FLAG_TRUSTED: u32 = 2;

pub(crate) fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub(crate) fn check_timestamp(timestamp: f64, now: f64) -> Result<(), OverlayError> {
    if !timestamp.is_finite()
        || timestamp < now - PLUMTREE_TIMESTAMP_WINDOW_SECONDS
        || timestamp > now + PLUMTREE_TIMESTAMP_WINDOW_SECONDS
    {
        return Err(OverlayError::InvalidPlumtreePayload(
            "timestamp is outside the accepted window".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn check_control_fields(
    broadcast_id: &Int256,
    part_index: i32,
    tree_index: i32,
    config: &PlumtreeConfig,
) -> Result<(), OverlayError> {
    if broadcast_id.0 == [0; 32] {
        return Err(OverlayError::InvalidPlumtreePayload(
            "empty broadcast id".to_owned(),
        ));
    }
    let Ok(part_index) = u32::try_from(part_index) else {
        return Err(OverlayError::InvalidPlumtreePayload(
            "negative part index".to_owned(),
        ));
    };
    let Ok(tree_index) = u32::try_from(tree_index) else {
        return Err(OverlayError::InvalidPlumtreePayload(
            "negative tree index".to_owned(),
        ));
    };
    if tree_index >= config.tree_slots {
        return Err(OverlayError::InvalidPlumtreePayload(
            "tree index is out of range".to_owned(),
        ));
    }
    if tree_index == PLUMTREE_SIMPLE_TREE_INDEX {
        if part_index != 0 {
            return Err(OverlayError::InvalidPlumtreePayload(
                "simple broadcast uses a non-zero part index".to_owned(),
            ));
        }
        return Ok(());
    }
    if part_index >= config.parts || tree_index != part_index + PLUMTREE_FEC_TREE_OFFSET {
        return Err(OverlayError::InvalidPlumtreePayload(
            "FEC part and tree indices do not match".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn sha256(data: &[u8]) -> Int256 {
    Int256(Sha256::digest(data).into())
}

pub(crate) fn fec_id(
    flags: i32,
    full_data_hash: Int256,
    full_data_size: i32,
    part_size: i32,
) -> Int256 {
    sha256(&tl_proto::serialize(OverlayBroadcastPlumtreeFecId {
        flags,
        full_data_hash,
        full_data_size,
        part_size,
    }))
}

pub(crate) fn simple_to_sign(
    id: Int256,
    timestamp: f64,
    tree_index: i32,
    data_size: i32,
    data_hash: Int256,
) -> Vec<u8> {
    tl_proto::serialize(OverlayBroadcastPlumtreeSimpleToSign {
        id,
        timestamp: timestamp.into(),
        tree_index,
        data_size,
        data_hash,
    })
}

pub(crate) fn fec_to_sign(
    id: Int256,
    timestamp: f64,
    part_index: i32,
    tree_index: i32,
    data_size: i32,
    data_hash: Int256,
) -> Vec<u8> {
    tl_proto::serialize(OverlayBroadcastPlumtreeFecToSign {
        id,
        timestamp: timestamp.into(),
        part_index,
        tree_index,
        data_size,
        data_hash,
    })
}

pub(crate) fn source_key(src: &TlPublicKey) -> Result<[u8; 32], OverlayError> {
    let TlPublicKey::Ed25519 { key } = src else {
        return Err(OverlayError::InvalidPlumtreePayload(
            "Plumtree source is not an Ed25519 key".to_owned(),
        ));
    };
    Ok(key.0)
}

pub(crate) fn source_hash(src: &TlPublicKey) -> Result<Int256, OverlayError> {
    let key = source_key(src)?;
    let mut boxed = 0x4813b4c6u32.to_le_bytes().to_vec();
    boxed.extend_from_slice(&key);
    Ok(sha256(&boxed))
}

/// Flags upstream assigns to a V1 certificate from its `max_size`.
fn default_certificate_flags(max_size: i32) -> u32 {
    let allow_fec = if i32::try_from(SIMPLE_BROADCAST_MAX).is_ok_and(|limit| max_size > limit) {
        CERT_FLAG_ALLOW_FEC
    } else {
        0
    };
    allow_fec | CERT_FLAG_TRUSTED
}

/// Serializes the `overlay.certificateId`/`overlay.certificateIdV2`
/// preimage an overlay certificate signs for `node`.
pub(crate) fn certificate_to_sign(
    overlay: OverlayId,
    node: Int256,
    expire_at: i32,
    max_size: i32,
    flags: u32,
) -> Vec<u8> {
    if flags == default_certificate_flags(max_size) {
        tl_proto::serialize(OverlayCertificateId {
            overlay_id: Int256(overlay.as_bytes()),
            node,
            expire_at,
            max_size,
        })
    } else {
        tl_proto::serialize(OverlayCertificateIdV2 {
            overlay_id: Int256(overlay.as_bytes()),
            node,
            expire_at,
            max_size,
            flags: flags.cast_signed(),
        })
    }
}

/// Checks the minimal Plumtree certificate eligibility rules: expiry,
/// size, FEC permission, and the issuer's Ed25519 signature over the
/// certificate id.  Private-overlay privacy rules are not modelled.
pub(crate) fn check_certificate(
    overlay: OverlayId,
    source: Int256,
    certificate: &OverlayCertificate,
    size: u32,
    is_fec: bool,
    now: i32,
) -> Result<(), OverlayError> {
    let (issued_by, expire_at, max_size, flags, signature) = match certificate {
        OverlayCertificate::Empty => return Ok(()),
        OverlayCertificate::Certificate {
            issued_by,
            expire_at,
            max_size,
            signature,
        } => (
            issued_by,
            *expire_at,
            *max_size,
            default_certificate_flags(*max_size),
            signature,
        ),
        OverlayCertificate::CertificateV2 {
            issued_by,
            expire_at,
            max_size,
            flags,
            signature,
        } => (
            issued_by,
            *expire_at,
            *max_size,
            flags.cast_unsigned(),
            signature,
        ),
    };
    if max_size < 0 || size > max_size.cast_unsigned() {
        return Err(OverlayError::InvalidPlumtreePayload(
            "Plumtree certificate size is too small".to_owned(),
        ));
    }
    if now > expire_at {
        return Err(OverlayError::InvalidPlumtreePayload(
            "Plumtree certificate has expired".to_owned(),
        ));
    }
    if is_fec && flags & CERT_FLAG_ALLOW_FEC == 0 {
        return Err(OverlayError::InvalidPlumtreePayload(
            "Plumtree certificate does not allow FEC".to_owned(),
        ));
    }
    let TlPublicKey::Ed25519 { key } = issued_by else {
        return Err(OverlayError::InvalidPlumtreePayload(
            "Plumtree certificate issuer is not an Ed25519 key".to_owned(),
        ));
    };
    let preimage = certificate_to_sign(overlay, source, expire_at, max_size, flags);
    verify(key.0, &preimage, signature).map_err(|_| {
        OverlayError::InvalidPlumtreePayload("Plumtree certificate signature is invalid".to_owned())
    })
}

/// Builds the `overlay.query{overlay}` wrapper around an
/// `overlay.repairPlumtreePart` request.
pub(crate) fn repair_query_wire(overlay: OverlayId, key: RepairKey) -> Vec<u8> {
    let (broadcast_id, part_index, tree_index) = key;
    let mut query = tl_proto::serialize(OverlayQuery::Query {
        overlay: Int256(overlay.as_bytes()),
    });
    query.extend(tl_proto::serialize(OverlayRepairPlumtreePart {
        broadcast_id: Int256(broadcast_id),
        timestamp: unix_seconds().into(),
        part_index: part_index.cast_signed(),
        tree_index: tree_index.cast_signed(),
    }));
    query
}

pub(crate) fn verify(
    source: [u8; 32],
    preimage: &[u8],
    signature: &[u8],
) -> Result<(), OverlayError> {
    let signature: [u8; 64] = signature.try_into().map_err(|_| {
        OverlayError::InvalidPlumtreePayload("invalid Plumtree signature length".to_owned())
    })?;
    let public_key = AdnlPublicKey::from_bytes(source).ok_or_else(|| {
        OverlayError::InvalidPlumtreePayload("invalid Plumtree source key".to_owned())
    })?;
    if public_key.verify_raw(preimage, &signature) {
        Ok(())
    } else {
        Err(OverlayError::InvalidPlumtreePayload(
            "invalid Plumtree signature".to_owned(),
        ))
    }
}

pub(crate) fn local_source(keypair: &AdnlKeyPair) -> TlPublicKey {
    TlPublicKey::Ed25519 {
        key: Int256(keypair.public_key.to_bytes()),
    }
}

pub(crate) fn simple_signed_part(
    keypair: &AdnlKeyPair,
    flags: i32,
    timestamp: f64,
    id: Int256,
    data: &[u8],
) -> Result<(Vec<u8>, PartInput), OverlayError> {
    let data_size = u32::try_from(data.len()).map_err(|_| {
        OverlayError::InvalidPlumtreePayload("Plumtree simple payload is too large".to_owned())
    })?;
    if data_size == 0 || data_size as usize > MAX_PLUMTREE_BROADCAST_SIZE {
        return Err(OverlayError::InvalidPlumtreePayload(
            "invalid Plumtree simple payload size".to_owned(),
        ));
    }
    let data_size = i32::try_from(data.len()).unwrap_or(i32::MAX);
    let data_hash = sha256(data);
    let signature = keypair.sign_raw(&simple_to_sign(
        id.clone(),
        timestamp,
        PLUMTREE_SIMPLE_TREE_INDEX.cast_signed(),
        data_size,
        data_hash.clone(),
    ));
    let part = PartInput {
        part_index: PLUMTREE_SIMPLE_TREE_INDEX,
        tree_index: PLUMTREE_SIMPLE_TREE_INDEX,
        timestamp,
        data_size: data_size.cast_unsigned(),
        data_hash: data_hash.clone(),
        source_key: local_source(keypair),
        certificate: OverlayCertificate::Empty,
        signature: signature.to_vec(),
        data: data.to_vec(),
    };
    let wire = tl_proto::serialize(crate::overlay::tl::OverlayBroadcast::PlumtreeSimple {
        flags,
        timestamp: timestamp.into(),
        src: local_source(keypair),
        certificate: crate::overlay::tl::OverlayCertificate::Empty,
        broadcast_id: id,
        tree_index: PLUMTREE_SIMPLE_TREE_INDEX.cast_signed(),
        data: data.to_vec(),
        signature: signature.to_vec(),
    });
    Ok((wire, part))
}

pub(crate) struct FecPartSpec {
    pub flags: i32,
    pub timestamp: f64,
    pub id: Int256,
    pub full_data_hash: Int256,
    pub full_data_size: i32,
    pub part_index: u32,
}

pub(crate) fn fec_signed_part(
    keypair: &AdnlKeyPair,
    spec: FecPartSpec,
    data: &[u8],
) -> Result<(Vec<u8>, PartInput), OverlayError> {
    let FecPartSpec {
        flags,
        timestamp,
        id,
        full_data_hash,
        full_data_size,
        part_index,
    } = spec;
    let data_size = u32::try_from(data.len()).map_err(|_| {
        OverlayError::InvalidPlumtreePayload("Plumtree FEC part is too large".to_owned())
    })?;
    if data_size == 0 || full_data_size <= 0 {
        return Err(OverlayError::InvalidPlumtreePayload(
            "invalid Plumtree FEC part size".to_owned(),
        ));
    }
    let tree_index = part_index + PLUMTREE_FEC_TREE_OFFSET;
    let data_hash = sha256(data);
    let signature = keypair.sign_raw(&fec_to_sign(
        id.clone(),
        timestamp,
        part_index.cast_signed(),
        tree_index.cast_signed(),
        data.len().try_into().unwrap_or(i32::MAX),
        data_hash.clone(),
    ));
    let part = PartInput {
        part_index,
        tree_index,
        timestamp,
        data_size,
        data_hash: data_hash.clone(),
        source_key: local_source(keypair),
        certificate: OverlayCertificate::Empty,
        signature: signature.to_vec(),
        data: data.to_vec(),
    };
    let wire = tl_proto::serialize(crate::overlay::tl::OverlayBroadcast::PlumtreeFec {
        flags,
        timestamp: timestamp.into(),
        src: local_source(keypair),
        certificate: crate::overlay::tl::OverlayCertificate::Empty,
        full_data_hash,
        full_data_size,
        part_index: part_index.cast_signed(),
        tree_index: tree_index.cast_signed(),
        data: data.to_vec(),
        signature: signature.to_vec(),
    });
    Ok((wire, part))
}

pub(crate) struct FecEncoding {
    #[cfg(test)]
    pub full_data_size: u32,
    pub part_size: u32,
    pub source_count: u32,
    pub full_data_hash: Int256,
    pub broadcast_id: Int256,
    pub symbols: Vec<Vec<u8>>,
}

pub(crate) fn encode_fec(
    data: &[u8],
    flags: i32,
    config: &PlumtreeConfig,
) -> Result<FecEncoding, OverlayError> {
    let full_data_size = u32::try_from(data.len()).map_err(|_| {
        OverlayError::InvalidPlumtreePayload("Plumtree payload is too large".to_owned())
    })?;
    let part_size = full_data_size.div_ceil(config.k);
    if part_size == 0 || part_size > u32::from(u16::MAX) {
        return Err(OverlayError::InvalidPlumtreePayload(
            "Plumtree FEC part size is unsupported by RaptorQ".to_owned(),
        ));
    }
    let full_data_hash = sha256(data);
    let part_size_i32 = part_size.cast_signed();
    let full_data_size_i32 = full_data_size.cast_signed();
    let broadcast_id = fec_id(
        flags,
        full_data_hash.clone(),
        full_data_size_i32,
        part_size_i32,
    );
    let encoder = Encoder::new(
        data,
        ObjectTransmissionInformation::new(
            u64::from(full_data_size),
            part_size.try_into().map_err(|_| {
                OverlayError::InvalidPlumtreePayload("invalid RaptorQ symbol size".to_owned())
            })?,
            1,
            1,
            1,
        ),
    );
    let block = &encoder.get_block_encoders()[0];
    let source = block
        .source_packets()
        .into_iter()
        .map(|packet| packet.split().1);
    let source_count = u32::try_from(source.len()).map_err(|_| {
        OverlayError::InvalidPlumtreePayload("Plumtree source symbol count overflow".to_owned())
    })?;
    let repair_count = config.parts.saturating_sub(source_count);
    let repair = block
        .repair_packets(0, repair_count)
        .into_iter()
        .map(|packet| packet.split().1);
    let symbols = source.chain(repair).take(config.parts as usize).collect();
    Ok(FecEncoding {
        #[cfg(test)]
        full_data_size,
        part_size,
        source_count,
        full_data_hash,
        broadcast_id,
        symbols,
    })
}

pub(crate) fn new_fec_decoder(full_size: u32, part_size: u32) -> Result<Decoder, OverlayError> {
    let symbol_size = u16::try_from(part_size).map_err(|_| {
        OverlayError::InvalidPlumtreePayload("Plumtree FEC part is too large".to_owned())
    })?;
    Ok(Decoder::new(ObjectTransmissionInformation::new(
        u64::from(full_size),
        symbol_size,
        1,
        1,
        1,
    )))
}

pub(crate) fn fec_packet(part_index: u32, data: Vec<u8>, source_parts: u32) -> EncodingPacket {
    EncodingPacket::new(
        raptorq::PayloadId::new(
            0,
            crate::overlay::fec_wire_symbol_id_to_internal(part_index, source_parts),
        ),
        data,
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        adnl::{AdnlKeyPair, now_i32},
        overlay::{OverlayId, inbound::parse_repair_query, tl::OverlayBroadcast},
        tl::TlInt256 as Int256,
    };

    use super::{
        CERT_FLAG_ALLOW_FEC, OverlayCertificate, PlumtreeConfig, check_certificate,
        check_timestamp, default_certificate_flags, encode_fec, fec_packet, new_fec_decoder,
        repair_query_wire, sha256, simple_signed_part, simple_to_sign, source_key, unix_seconds,
        verify,
    };

    fn keypair() -> AdnlKeyPair {
        AdnlKeyPair::generate(&mut rand::rngs::OsRng)
    }

    fn certificate(
        issuer: &AdnlKeyPair,
        overlay: OverlayId,
        node: &Int256,
        expire_at: i32,
        max_size: i32,
        flags: u32,
    ) -> OverlayCertificate {
        let preimage =
            super::certificate_to_sign(overlay, node.clone(), expire_at, max_size, flags);
        let signature = issuer.sign_raw(&preimage).to_vec();
        let issued_by = super::local_source(issuer);
        if flags == default_certificate_flags(max_size) {
            OverlayCertificate::Certificate {
                issued_by,
                expire_at,
                max_size,
                signature,
            }
        } else {
            OverlayCertificate::CertificateV2 {
                issued_by,
                expire_at,
                max_size,
                flags: flags.cast_signed(),
                signature,
            }
        }
    }

    #[test]
    fn timestamp_check_enforces_the_upstream_window() {
        let now = unix_seconds();
        check_timestamp(now, now).unwrap();
        check_timestamp(now - 20.0, now).unwrap();
        check_timestamp(now + 20.0, now).unwrap();
        check_timestamp(now - 19.0, now).unwrap();
        assert!(check_timestamp(now - 21.0, now).is_err());
        assert!(check_timestamp(now + 21.0, now).is_err());
        assert!(check_timestamp(f64::NAN, now).is_err());
        assert!(check_timestamp(f64::INFINITY, now).is_err());
        assert!(check_timestamp(f64::NEG_INFINITY, now).is_err());
    }

    #[test]
    fn control_field_bounds_match_the_configured_trees_and_parts() {
        let config = PlumtreeConfig {
            k: 4,
            parts: 6,
            tree_slots: 7,
            ..PlumtreeConfig::default()
        };
        super::check_control_fields(&Int256([1; 32]), 0, 0, &config).unwrap();
        super::check_control_fields(&Int256([1; 32]), 5, 6, &config).unwrap();
        assert!(super::check_control_fields(&Int256([0; 32]), 0, 0, &config).is_err());
        assert!(super::check_control_fields(&Int256([1; 32]), 1, 0, &config).is_err());
        assert!(super::check_control_fields(&Int256([1; 32]), 0, -1, &config).is_err());
        assert!(super::check_control_fields(&Int256([1; 32]), -1, 0, &config).is_err());
        assert!(super::check_control_fields(&Int256([1; 32]), 6, 7, &config).is_err());
        assert!(super::check_control_fields(&Int256([1; 32]), 0, 7, &config).is_err());
        assert!(super::check_control_fields(&Int256([1; 32]), 1, 3, &config).is_err());
    }

    #[test]
    fn empty_and_oversized_payloads_are_rejected_before_signing() {
        let keypair = keypair();
        let config = PlumtreeConfig {
            k: 4,
            parts: 6,
            tree_slots: 7,
            ..PlumtreeConfig::default()
        };
        assert!(simple_signed_part(&keypair, 0, unix_seconds(), Int256([1; 32]), &[]).is_err());
        assert!(encode_fec(&[], 0, &config).is_err());
        assert!(
            super::fec_signed_part(
                &keypair,
                super::FecPartSpec {
                    flags: 0,
                    timestamp: unix_seconds(),
                    id: Int256([1; 32]),
                    full_data_hash: sha256(b"payload"),
                    full_data_size: 0,
                    part_index: 0,
                },
                b"part",
            )
            .is_err()
        );
    }

    #[test]
    fn signed_simple_part_roundtrips_and_rejects_tampering() {
        let keypair = keypair();
        let id = Int256([7; 32]);
        let timestamp = unix_seconds();
        let data = b"plumtree simple payload";
        let (wire, part) = simple_signed_part(&keypair, 6, timestamp, id.clone(), data).unwrap();
        let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&wire).unwrap();
        let OverlayBroadcast::PlumtreeSimple {
            flags,
            timestamp: wire_timestamp,
            src,
            broadcast_id,
            tree_index,
            data: ref wire_data,
            ref signature,
            ..
        } = broadcast
        else {
            panic!("simple part must serialize as overlay.broadcastPlumtreeSimple");
        };
        assert_eq!(flags, 6);
        assert_eq!(wire_timestamp.to_f64(), timestamp);
        assert_eq!(broadcast_id, id);
        assert_eq!(tree_index, 0);
        assert_eq!(wire_data, data);
        assert_eq!(part.data_size, u32::try_from(data.len()).unwrap());
        assert_eq!(part.data_hash, sha256(data));

        let source = source_key(&src).unwrap();
        let preimage = simple_to_sign(
            broadcast_id.clone(),
            wire_timestamp.to_f64(),
            tree_index,
            i32::try_from(wire_data.len()).unwrap(),
            sha256(wire_data),
        );
        verify(source, &preimage, signature).unwrap();
        assert!(
            verify(
                source,
                &simple_to_sign(
                    broadcast_id,
                    wire_timestamp.to_f64(),
                    tree_index,
                    3,
                    sha256(b"bad"),
                ),
                signature,
            )
            .is_err()
        );
    }

    #[test]
    fn fec_encoding_decodes_back_to_the_payload() {
        let data = (0..500u32)
            .map(|value| u8::try_from(value % 251).unwrap())
            .collect::<Vec<_>>();
        let config = PlumtreeConfig {
            k: 4,
            parts: 6,
            tree_slots: 7,
            ..PlumtreeConfig::default()
        };
        let encoding = encode_fec(&data, 0, &config).unwrap();
        assert_eq!(encoding.symbols.len(), config.parts as usize);
        assert_eq!(encoding.source_count, 4);
        assert_eq!(
            encoding.broadcast_id,
            super::fec_id(
                0,
                encoding.full_data_hash.clone(),
                i32::try_from(data.len()).unwrap(),
                i32::try_from(encoding.part_size).unwrap(),
            )
        );

        let mut decoder =
            new_fec_decoder(u32::try_from(data.len()).unwrap(), encoding.part_size).unwrap();
        let decoded = (0..encoding.source_count)
            .find_map(|index| {
                decoder.decode(fec_packet(
                    index,
                    encoding.symbols[index as usize].clone(),
                    encoding.source_count,
                ))
            })
            .expect("source symbols must decode the payload");
        assert_eq!(decoded, data);
    }

    #[test]
    fn certificate_validation_enforces_size_expiry_and_fec_flag() {
        let issuer = keypair();
        let overlay = OverlayId::from_name(b"plumtree-certificate");
        let source = Int256([3; 32]);
        let now = now_i32();
        let max_size = 2048;
        let v1_fec = certificate(
            &issuer,
            overlay,
            &source,
            now + 60,
            max_size,
            default_certificate_flags(max_size),
        );
        check_certificate(overlay, source.clone(), &v1_fec, 1024, true, now).unwrap();
        assert!(
            check_certificate(overlay, source.clone(), &v1_fec, 4096, true, now).is_err(),
            "payload size above max_size must be rejected"
        );
        assert!(
            check_certificate(overlay, source.clone(), &v1_fec, 1024, true, now + 61).is_err(),
            "expired certificates must be rejected"
        );

        let simple_only = certificate(
            &issuer,
            overlay,
            &source,
            now + 60,
            512,
            default_certificate_flags(512),
        );
        assert!(
            check_certificate(overlay, source.clone(), &simple_only, 512, true, now).is_err(),
            "V1 certificates without AllowFec must reject FEC parts"
        );
        check_certificate(overlay, source.clone(), &simple_only, 512, false, now).unwrap();

        let explicit_fec = certificate(
            &issuer,
            overlay,
            &source,
            now + 60,
            1024,
            CERT_FLAG_ALLOW_FEC,
        );
        check_certificate(overlay, source.clone(), &explicit_fec, 512, true, now).unwrap();

        let mut tampered = simple_only;
        if let OverlayCertificate::Certificate { signature, .. } = &mut tampered {
            signature[0] ^= 0xff;
        } else {
            panic!("expected a V1 certificate");
        }
        assert!(
            check_certificate(overlay, source, &tampered, 512, false, now).is_err(),
            "tampered certificate signatures must be rejected"
        );
    }

    #[test]
    fn repair_query_wraps_the_overlay_and_roundtrips() {
        let overlay = OverlayId::from_name(b"plumtree-repair");
        let key = ([5u8; 32], 3, 4);
        let query = repair_query_wire(overlay, key);
        let parsed = parse_repair_query(&query, overlay).expect("wrapped repair query must parse");
        assert_eq!(parsed.broadcast_id.0, key.0);
        assert_eq!(parsed.part_index, 3);
        assert_eq!(parsed.tree_index, 4);
        check_timestamp(parsed.timestamp.to_f64(), unix_seconds()).unwrap();
        assert!(
            parse_repair_query(&query, OverlayId::from_name(b"another-overlay")).is_none(),
            "a repair query for another overlay must not parse"
        );
    }
}
