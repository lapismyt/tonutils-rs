//! Overlay-direct external message broadcast construction.
//!
//! Upstream `FullNodeShardImpl::send_external_message`
//! (`validator/full-node-shard.cpp`) wraps the raw BoC into a
//! serialized `tonNode.externalMessageBroadcast` and hands it to
//! the overlay broadcast machinery: `BroadcastSimple` for payloads
//! that fit `Overlays::max_simple_broadcast_size()` (768 bytes)
//! and `BroadcastsFec` (RaptorQ, `overlay/broadcast-fec.cpp`)
//! for everything larger.  Both paths sign with this node's ADNL
//! key, and both propagate: every receiving node re-sends the
//! broadcast to its neighbours, which is what spreads a new
//! external message through the mempool overlay.

use std::sync::Arc;

use futures::future::{BoxFuture, join_all};
use raptorq::Encoder;
use sha2::{Digest, Sha256};
use tonutils_adnl::{KeyPair, now_i32};
use tonutils_overlay::PeerManager;
use tonutils_tl::Int256;
use tonutils_tl::tl::network::{
    FecType, OverlayBroadcast, OverlayBroadcastFec, OverlayBroadcastFecId,
    OverlayBroadcastFecPartId, OverlayBroadcastId, OverlayBroadcastToSign,
    PublicKey as TlPublicKey, TonNodeExternalMessage, TonNodeExternalMessageBroadcast,
};
use tonutils_tl::tl::overlay::OverlayCertificate;

use crate::BroadcastPeer;

/// Upstream `Overlays::max_simple_broadcast_size()`: the largest
/// payload a public overlay accepts as a signed simple broadcast.
///
/// Payloads above this size must go through FEC, whose upstream
/// cap is `Overlays::max_fec_broadcast_size()` (16 MiB).
pub const SIMPLE_BROADCAST_MAX: usize = 768;

/// RaptorQ symbol size upstream's `BroadcastFecActor` uses
/// (`overlay/broadcast-fec.cpp`).
pub const FEC_SYMBOL_SIZE: usize = 768;

/// Serializes the `tonNode.externalMessageBroadcast` wrapper that
/// upstream and pytoniq put into a broadcast's `data` field.
///
/// The raw BoC sits one wrapper below the broadcast payload, so a
/// receiver unwraps it after the broadcast itself is verified.
pub fn external_message_wrapper(raw_boc: &[u8]) -> Vec<u8> {
    tl_proto::serialize(TonNodeExternalMessageBroadcast {
        message: TonNodeExternalMessage {
            data: raw_boc.to_vec(),
        },
    })
}

/// Sender identity hash for a broadcast: `sha256` of the boxed
/// Ed25519 public key, or zero under the AnySender flag
/// (`overlay.broadcast.id` construction in
/// `overlay/broadcast-simple.cpp`).
fn broadcast_src(keypair: &KeyPair, flags: i32) -> Int256 {
    if flags & 1 != 0 {
        Int256([0u8; 32])
    } else {
        let mut boxed = 0x4813b4c6u32.to_le_bytes().to_vec();
        boxed.extend_from_slice(&keypair.public_key.to_bytes());
        Int256(Sha256::digest(boxed).into())
    }
}

/// Signs `data` as a simple broadcast the way `BroadcastSimple::create`
/// does (`overlay/broadcast-simple.cpp`): the signature covers
/// `overlay.broadcast.toSign{sha256(overlay.broadcast.id{src,
/// sha256(data), flags}), date}`.
pub fn signed_simple_broadcast(keypair: &KeyPair, data: Vec<u8>, flags: i32, date: i32) -> Vec<u8> {
    let public_key = keypair.public_key.to_bytes();
    let src = broadcast_src(keypair, flags);
    let broadcast_id = tl_proto::serialize(OverlayBroadcastId {
        src,
        data_hash: Int256(Sha256::digest(&data).into()),
        flags,
    });
    let signature = keypair.sign_raw(&tl_proto::serialize(OverlayBroadcastToSign {
        hash: Int256(Sha256::digest(broadcast_id).into()),
        date,
    }));
    tl_proto::serialize(OverlayBroadcast::Broadcast {
        src: TlPublicKey::Ed25519 {
            key: Int256(public_key),
        },
        certificate: OverlayCertificate::Empty,
        flags,
        data,
        date,
        signature: signature.to_vec(),
    })
}

/// One signed `overlay.broadcastFec` part of a FEC broadcast.
pub struct FecBroadcastPart {
    /// Serialized `overlay.broadcastFec` object, ready to ride
    /// after the `overlay.message` header.
    pub object: Vec<u8>,
    /// RaptorQ symbol id this part carries.
    pub seqno: i32,
}

/// Identity of one FEC broadcast, shared by every part.
struct FecBroadcastIdentity {
    src: Int256,
    fec_type: FecType,
    data_hash: Int256,
    data_size: i32,
    flags: i32,
}

/// Splits `data` into signed RaptorQ broadcast parts.
///
/// Each part carries the raw RaptorQ symbol bytes in its `data`
/// field and the symbol id in `seqno` - the wire format
/// `BroadcastsFec::process_broadcast` (`overlay/broadcast-fec.cpp`)
/// decodes.  Every part is signed over its own
/// `overlay.broadcastFec.partId`, which upstream's
/// `BroadcastFecPart::run_checks` verifies before the symbol is
/// fed to the decoder.
///
/// Upstream paces symbol emission over time (`BroadcastFecActor`
/// emits four symbols per 10 ms tick); this function returns every
/// source symbol at once so the caller can fire the whole fan-out
/// in a single round of datagrams.  Receivers reconstruct the
/// object from any `symbols_count` symbols, so the source set
/// alone is sufficient.
pub fn fec_broadcast_parts(
    keypair: &KeyPair,
    data: &[u8],
    flags: i32,
    date: i32,
) -> Vec<FecBroadcastPart> {
    let data_size = data.len() as i32;
    let symbol_size = FEC_SYMBOL_SIZE as i32;
    let symbols_count = data.len().div_ceil(FEC_SYMBOL_SIZE) as i32;
    let fec_type = FecType::RaptorQ {
        data_size,
        symbol_size,
        symbols_count,
    };
    let data_hash = Int256(Sha256::digest(data).into());
    let encoder = Encoder::with_defaults(data, FEC_SYMBOL_SIZE as u16);
    let public_key = keypair.public_key.to_bytes();
    let identity = FecBroadcastIdentity {
        src: broadcast_src(keypair, flags),
        fec_type,
        data_hash,
        data_size,
        flags,
    };
    encoder
        .get_encoded_packets(0)
        .into_iter()
        .map(|packet| {
            let seqno = packet.payload_id().encoding_symbol_id() as i32;
            let symbol = packet.data();
            let signature = fec_part_signature(keypair, &identity, symbol, seqno, date);
            let object = tl_proto::serialize(OverlayBroadcastFec {
                src: TlPublicKey::Ed25519 {
                    key: Int256(public_key),
                },
                certificate: OverlayCertificate::Empty,
                data_hash: identity.data_hash.clone(),
                data_size,
                flags,
                data: symbol.to_vec(),
                seqno,
                fec: identity.fec_type.clone(),
                date,
                signature,
            });
            FecBroadcastPart { object, seqno }
        })
        .collect()
}

/// Signs one FEC part the way upstream's `BroadcastsFec::send_part`
/// does: the signature covers
/// `overlay.broadcast.toSign{sha256(overlay.broadcastFec.partId{
/// sha256(overlay.broadcastFec.id{src, sha256(fec.Type),
/// sha256(data), size, flags}), sha256(symbol), seqno}), date}`.
fn fec_part_signature(
    keypair: &KeyPair,
    identity: &FecBroadcastIdentity,
    symbol: &[u8],
    seqno: i32,
    date: i32,
) -> Vec<u8> {
    let fec_type_hash =
        Int256(Sha256::digest(tl_proto::serialize(identity.fec_type.clone())).into());
    let broadcast_id = tl_proto::serialize(OverlayBroadcastFecId {
        src: identity.src.clone(),
        fec_type_hash,
        data_hash: identity.data_hash.clone(),
        size: identity.data_size,
        flags: identity.flags,
    });
    let broadcast_hash = Int256(Sha256::digest(broadcast_id).into());
    let part_data_hash = Int256(Sha256::digest(symbol).into());
    let part_id = tl_proto::serialize(OverlayBroadcastFecPartId {
        broadcast_hash,
        part_data_hash,
        seqno,
    });
    let part_hash = Int256(Sha256::digest(part_id).into());
    keypair
        .sign_raw(&tl_proto::serialize(OverlayBroadcastToSign {
            hash: part_hash,
            date,
        }))
        .to_vec()
}

/// Broadcast wire format selected for one external message.
pub enum ExternalMessageBroadcast {
    /// Signed simple broadcast - one datagram per peer.
    Simple(Vec<u8>),
    /// Signed RaptorQ source symbols, one `overlay.broadcastFec`
    /// object each.
    Fec(Vec<FecBroadcastPart>),
}

/// Wraps an external message BoC and signs it for the overlay,
/// picking the simple broadcast path for payloads that fit
/// [`SIMPLE_BROADCAST_MAX`] and the FEC path for the rest.
pub fn external_message_broadcast(
    keypair: &KeyPair,
    raw_boc: &[u8],
    flags: i32,
    date: i32,
) -> ExternalMessageBroadcast {
    let wrapper = external_message_wrapper(raw_boc);
    if wrapper.len() <= SIMPLE_BROADCAST_MAX {
        ExternalMessageBroadcast::Simple(signed_simple_broadcast(keypair, wrapper, flags, date))
    } else {
        ExternalMessageBroadcast::Fec(fec_broadcast_parts(keypair, &wrapper, flags, date))
    }
}

/// [`BroadcastPeer`] that publishes external messages straight onto
/// the overlay every scanner session is connected to.
///
/// The signed broadcast (or, for payloads above
/// [`SIMPLE_BROADCAST_MAX`], every signed RaptorQ source symbol) is
/// serialized once and the identical bytes are fanned out to all
/// UDP and QUIC sessions in parallel, so a published message
/// leaves this node in a single round of datagrams and keeps
/// propagating through upstream's neighbour re-broadcast.
pub struct OverlayBroadcastPeer {
    manager: Arc<PeerManager>,
    keypair: KeyPair,
    flags: i32,
}

impl OverlayBroadcastPeer {
    /// Creates a broadcast peer for the overlay `manager` carries
    /// sessions for, signing broadcasts with `keypair`.
    pub fn new(manager: Arc<PeerManager>, keypair: KeyPair) -> Self {
        Self {
            manager,
            keypair,
            flags: 0,
        }
    }

    /// Sets the broadcast flags.  Flag 1 (`BroadcastFlagAnySender`)
    /// zeroes the `src` field so any relay may re-broadcast; the
    /// default of `0` attributes the broadcast to this node's key,
    /// which upstream receivers verify against `src`.
    pub fn with_flags(mut self, flags: i32) -> Self {
        self.flags = flags;
        self
    }
}

impl BroadcastPeer for OverlayBroadcastPeer {
    fn send_external(&self, raw_boc: Arc<[u8]>) -> BoxFuture<'static, Result<(), String>> {
        let manager = self.manager.clone();
        let keypair = self.keypair;
        let flags = self.flags;
        Box::pin(async move {
            let date = now_i32();
            match external_message_broadcast(&keypair, &raw_boc, flags, date) {
                ExternalMessageBroadcast::Simple(object) => {
                    let sent = manager.broadcast_object(Arc::from(object)).await;
                    if sent == 0 {
                        return Err("no overlay sessions connected".to_owned());
                    }
                    log::debug!("overlay broadcast: simple sent to {sent} session(s)");
                    Ok(())
                }
                ExternalMessageBroadcast::Fec(parts) => {
                    let total = parts.len();
                    let results = join_all(parts.into_iter().map(|part| {
                        let manager = manager.clone();
                        let object = Arc::from(part.object);
                        async move { manager.broadcast_object(object).await }
                    }))
                    .await;
                    let delivered = results.into_iter().filter(|sent| *sent > 0).count();
                    if delivered == 0 {
                        return Err("no overlay sessions connected".to_owned());
                    }
                    log::debug!(
                        "overlay broadcast: FEC delivered {delivered}/{total} part fan-outs"
                    );
                    Ok(())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BoC-shaped payload of `size` bytes: the BoC magic plus
    /// filler, long enough for the envelope checks to pass.
    fn test_boc(size: usize) -> Vec<u8> {
        let mut boc = vec![0xb5, 0xee, 0x9c, 0x72];
        boc.resize(size, 0x41);
        boc
    }

    #[test]
    fn simple_broadcast_verifies_through_payload_if_valid() {
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
        let data = test_boc(64);
        let date = now_i32();
        let signed = signed_simple_broadcast(&keypair, data.clone(), 0, date);
        let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&signed).expect("valid TL");
        assert_eq!(
            broadcast.payload_if_valid(date),
            Some(&data[..]),
            "a correctly signed broadcast must unwrap to its payload"
        );
        // A date more than 60 seconds in the future is rejected by
        // upstream's date check.
        let future = signed_simple_broadcast(&keypair, data.clone(), 0, date + 600);
        let future = tl_proto::deserialize::<OverlayBroadcast>(&future).expect("valid TL");
        assert!(future.payload_if_valid(date).is_none());
        // Tampering the signature breaks verification.  The bytes
        // field carries zero padding, so the tamper goes through
        // the typed field instead of a raw byte offset.
        let mut tampered = tl_proto::deserialize::<OverlayBroadcast>(&signed).expect("valid TL");
        if let OverlayBroadcast::Broadcast { signature, .. } = &mut tampered {
            signature[0] ^= 0xff;
        }
        let tampered = tl_proto::deserialize::<OverlayBroadcast>(&tl_proto::serialize(tampered))
            .expect("still valid TL");
        assert!(tampered.payload_if_valid(date).is_none());
    }

    #[test]
    fn any_sender_flag_broadcast_still_verifies() {
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
        let data = test_boc(32);
        let date = now_i32();
        // Under the AnySender flag the broadcast id's src is zeroed,
        // so the signature covers the zeroed id - payload_if_valid
        // recomputes exactly that preimage.
        let signed = signed_simple_broadcast(&keypair, data.clone(), 1, date);
        let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&signed).expect("valid TL");
        assert_eq!(broadcast.payload_if_valid(date), Some(&data[..]));
    }

    #[test]
    fn size_threshold_selects_simple_or_fec() {
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
        let date = now_i32();
        // The wrapper adds the boxed constructor, the bytes
        // length prefix and zero padding, so walk down from the
        // cap to the BoC size whose wrapper exactly fills it.
        let mut boc_len = SIMPLE_BROADCAST_MAX;
        while external_message_wrapper(&test_boc(boc_len)).len() > SIMPLE_BROADCAST_MAX {
            boc_len -= 1;
        }
        let at_cap = external_message_broadcast(&keypair, &test_boc(boc_len), 0, date);
        assert!(
            matches!(at_cap, ExternalMessageBroadcast::Simple(_)),
            "a wrapper of {} bytes must stay a simple broadcast",
            external_message_wrapper(&test_boc(boc_len)).len()
        );
        let over_cap = external_message_broadcast(&keypair, &test_boc(boc_len + 1), 0, date);
        assert!(
            matches!(over_cap, ExternalMessageBroadcast::Fec(_)),
            "a wrapper of {} bytes must switch to FEC",
            external_message_wrapper(&test_boc(boc_len + 1)).len()
        );
    }

    #[test]
    fn fec_parts_encode_decode_and_carry_valid_signatures() {
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
        let date = now_i32();
        let wrapper = external_message_wrapper(&test_boc(2000));
        let parts = fec_broadcast_parts(&keypair, &wrapper, 0, date);
        let symbols_count = wrapper.len().div_ceil(FEC_SYMBOL_SIZE);
        assert_eq!(parts.len(), symbols_count, "one part per source symbol");

        let data_hash: [u8; 32] = Sha256::digest(&wrapper).into();
        let mut symbols = Vec::with_capacity(parts.len());
        for (index, part) in parts.iter().enumerate() {
            assert_eq!(part.seqno, index as i32, "source symbols are ordered");
            let object = tl_proto::deserialize::<OverlayBroadcastFec>(&part.object)
                .expect("valid FEC object");
            assert_eq!(
                object.data_hash.0, data_hash,
                "every part names the object hash"
            );
            assert_eq!(object.data_size as usize, wrapper.len());
            assert_eq!(
                object.data.len(),
                FEC_SYMBOL_SIZE,
                "raw upstream symbol format"
            );
            assert_eq!(object.seqno, index as i32);
            match &object.fec {
                FecType::RaptorQ {
                    data_size,
                    symbol_size,
                    symbols_count,
                } => {
                    assert_eq!(*data_size as usize, wrapper.len());
                    assert_eq!(*symbol_size as usize, FEC_SYMBOL_SIZE);
                    assert_eq!(*symbols_count as usize, parts.len());
                }
                _ => panic!("expected RaptorQ"),
            }
            // The part signature must verify over the reconstructed
            // part id, exactly as upstream's BroadcastFecPart::run_checks
            // recomputes it.
            let fec_type_hash: [u8; 32] =
                Sha256::digest(tl_proto::serialize(object.fec.clone())).into();
            let broadcast_id = tl_proto::serialize(OverlayBroadcastFecId {
                src: broadcast_src(&keypair, 0),
                fec_type_hash: Int256(fec_type_hash),
                data_hash: Int256(data_hash),
                size: object.data_size,
                flags: 0,
            });
            let broadcast_hash: [u8; 32] = Sha256::digest(&broadcast_id).into();
            let part_data_hash: [u8; 32] = Sha256::digest(&object.data).into();
            let part_id = tl_proto::serialize(OverlayBroadcastFecPartId {
                broadcast_hash: Int256(broadcast_hash),
                part_data_hash: Int256(part_data_hash),
                seqno: object.seqno,
            });
            let part_hash: [u8; 32] = Sha256::digest(&part_id).into();
            let to_sign = tl_proto::serialize(OverlayBroadcastToSign {
                hash: Int256(part_hash),
                date,
            });
            let expected = keypair.sign_raw(&to_sign);
            assert_eq!(object.signature, expected.to_vec());
            symbols.push((object.seqno as u32, object.data.clone()));
        }

        // The source symbols must decode back to the wrapper, each
        // under its own symbol id.
        let mut decoder = raptorq::Decoder::new(raptorq::ObjectTransmissionInformation::new(
            wrapper.len() as u64,
            FEC_SYMBOL_SIZE as u16,
            1,
            1,
            1,
        ));
        for (symbol_id, symbol) in symbols {
            let packet =
                raptorq::EncodingPacket::new(raptorq::PayloadId::new(0, symbol_id), symbol);
            if let Some(reconstructed) = decoder.decode(packet) {
                assert_eq!(reconstructed, wrapper);
                return;
            }
        }
        panic!("source symbols must reconstruct the object");
    }
}
