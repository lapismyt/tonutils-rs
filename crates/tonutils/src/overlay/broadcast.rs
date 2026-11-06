#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "ADNL and overlay wire fields and bounded counters intentionally use protocol widths"
)]
//! Signed simple and FEC overlay broadcast construction.
//!
//! The overlay layer is payload-agnostic. It selects `BroadcastSimple` for
//! payloads that fit `Overlays::max_simple_broadcast_size()` and
//! `BroadcastsFec` (RaptorQ, `overlay/broadcast-fec.cpp`) for larger ones.
//! Application protocols such as mempool wrap their own payloads before
//! calling [`OverlayBroadcastPeer::broadcast`].

use std::{sync::Arc, time::Duration};

use crate::{
    adnl::{AdnlKeyPair, now_i32, tl::AdnlTlPublicKey as TlPublicKey},
    overlay::{
        OverlayError, PeerManager, PeerQualityRegistry,
        protocol_stats::record_fec_part_sent,
        tl::{
            FecType, OverlayBroadcast, OverlayBroadcastFec, OverlayBroadcastFecId,
            OverlayBroadcastFecPartId, OverlayBroadcastId, OverlayBroadcastToSign,
            OverlayCertificate,
        },
    },
    tl::TlInt256 as Int256,
};

use futures::future::join_all;
use raptorq::{Encoder, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};

/// Upstream `Overlays::max_simple_broadcast_size()`: the largest
/// payload a public overlay accepts as a signed simple broadcast.
///
/// Payloads above this size must go through FEC, whose upstream
/// cap is `Overlays::max_fec_broadcast_size()` (16 MiB).
pub const SIMPLE_BROADCAST_MAX: usize = 768;

/// RaptorQ symbol size upstream's `BroadcastFecActor` uses
/// (`overlay/broadcast-fec.cpp`).
pub const FEC_SYMBOL_SIZE: usize = 768;

/// Hard cap on the `symbols_count` of an FEC broadcast part.
///
/// The raptorq decoder asserts the RFC 6330 `K'_max` table bound
/// (56 403 symbols per block) when its transmission information is
/// built, so a crafted `fec.raptorQ` type with a huge symbol count
/// would panic the receive loop.  Upstream broadcasts use
/// `size / 768 + 1` symbols over at most 1 MiB of data, far below
/// this bound.
pub(crate) const MAX_FEC_SOURCE_SYMBOLS: usize = 56_403;

/// Parts sent per FEC fan-out tick - upstream's
/// `BroadcastFecActor::alarm` emits four symbols per tick.
const FEC_PARTS_PER_TICK: usize = 4;

/// Spacing between FEC fan-out ticks - upstream's
/// `BroadcastFecActor::delay_` is 10 ms.
const FEC_SEND_TICK: Duration = Duration::from_millis(10);

/// Maps an `overlay.broadcastFec` wire `seqno` onto the raptorq
/// crate's internal encoding symbol id.
///
/// The wire space numbers source symbols `0..symbols_count` and then
/// repair symbols densely (`symbols_count + i`) - the space upstream's
/// `BroadcastFecActor` emits.  The raptorq crate instead reserves
/// `[symbols_count, K')` for the zero-padded extended symbols of the
/// RFC 6330 block and numbers repair symbols from
/// `K' = extended_source_block_symbols(symbols_count)`, so wire repair
/// ids shift up by `K' - symbols_count` before decoding - upstream's
/// `td::fec::raptorq::Decoder::add_symbol` applies the same shift
/// (`symbol.id += K_padded - K`).  Source ids pass through unchanged.
pub fn fec_wire_symbol_id_to_internal(wire_seqno: u32, symbols_count: u32) -> u32 {
    if wire_seqno < symbols_count {
        wire_seqno
    } else {
        wire_seqno + raptorq::extended_source_block_symbols(symbols_count) - symbols_count
    }
}

/// Sender identity hash for a broadcast: `sha256` of the boxed
/// Ed25519 public key, or zero under the AnySender flag
/// (`overlay.broadcast.id` construction in
/// `overlay/broadcast-simple.cpp`).
fn broadcast_src(keypair: &AdnlKeyPair, flags: i32) -> Int256 {
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
pub fn signed_simple_broadcast(
    keypair: &AdnlKeyPair,
    data: Vec<u8>,
    flags: i32,
    date: i32,
) -> Vec<u8> {
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

/// Splits `data` into signed RaptorQ broadcast parts: every source
/// symbol plus one repair symbol per source symbol.
///
/// Each part carries the raw RaptorQ symbol bytes in its `data`
/// field and the symbol id in `seqno` - the wire format
/// `BroadcastsFec::process_broadcast` (`overlay/broadcast-fec.cpp`)
/// decodes.  Every part is signed over its own
/// `overlay.broadcastFec.partId`, which upstream's
/// `BroadcastFecPart::run_checks` verifies before the symbol is
/// fed to the decoder.
///
/// Upstream's `BroadcastFecActor` emits `(size / 768 + 1) * 2`
/// symbols per broadcast: the `symbols_count` source symbols, which
/// are the raw data slices, followed by repair symbols above wire id
/// `symbols_count`.  This function emits the same set - a receiver
/// that loses any single datagram can still reconstruct from the
/// remaining parts instead of waiting for a retransmission that
/// never comes (broadcasts are not resent by the sender).  The
/// caller paces the fan-out at four parts per 10 ms tick the way
/// `BroadcastFecActor::alarm` does.
///
/// The raptorq crate numbers repair symbols from the extended block
/// size `K'`; see `fec_wire_symbol_id_to_internal` for the wire
/// renumbering applied here.
pub fn fec_broadcast_parts(
    keypair: &AdnlKeyPair,
    data: &[u8],
    flags: i32,
    date: i32,
) -> Vec<FecBroadcastPart> {
    if data.is_empty() {
        return Vec::new();
    }
    let data_size = data.len() as i32;
    let symbol_size = FEC_SYMBOL_SIZE as i32;
    let symbols_count = data.len().div_ceil(FEC_SYMBOL_SIZE) as i32;
    let fec_type = FecType::RaptorQ {
        data_size,
        symbol_size,
        symbols_count,
    };
    let data_hash = Int256(Sha256::digest(data).into());
    // The same single-block transmission information the receive
    // path builds, so encoder and decoder agree on the symbol
    // layout.
    let encoder = Encoder::new(
        data,
        ObjectTransmissionInformation::new(data.len() as u64, FEC_SYMBOL_SIZE as u16, 1, 1, 1),
    );
    let block = &encoder.get_block_encoders()[0];
    let public_key = keypair.public_key.to_bytes();
    let identity = FecBroadcastIdentity {
        src: broadcast_src(keypair, flags),
        fec_type,
        data_hash,
        data_size,
        flags,
    };
    let mut wire_symbols: Vec<(i32, Vec<u8>)> = Vec::with_capacity(2 * symbols_count as usize);
    for packet in block.source_packets() {
        let (payload_id, symbol) = packet.split();
        wire_symbols.push((payload_id.encoding_symbol_id() as i32, symbol));
    }
    for (index, packet) in block
        .repair_packets(0, symbols_count as u32)
        .into_iter()
        .enumerate()
    {
        // Repair symbols carry the dense upstream wire id
        // `symbols_count + i`; the raptorq crate's internal id starts
        // at the extended block size `K'`, so the offset is applied
        // on the wire (source wire ids equal the internal ids).
        let (_, symbol) = packet.split();
        wire_symbols.push((symbols_count + index as i32, symbol));
    }
    wire_symbols
        .into_iter()
        .map(|(seqno, symbol)| {
            let signature = fec_part_signature(keypair, &identity, &symbol, seqno, date);
            let object = tl_proto::serialize(OverlayBroadcastFec {
                src: TlPublicKey::Ed25519 {
                    key: Int256(public_key),
                },
                certificate: OverlayCertificate::Empty,
                data_hash: identity.data_hash.clone(),
                data_size,
                flags,
                data: symbol,
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
    keypair: &AdnlKeyPair,
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

/// Broadcast wire format selected for one payload.
pub enum SignedBroadcast {
    /// Signed simple broadcast - one datagram per peer.
    Simple(Vec<u8>),
    /// Signed RaptorQ symbols - every source symbol and one repair
    /// symbol each, one `overlay.broadcastFec` object each.
    Fec(Vec<FecBroadcastPart>),
}

/// Signs an overlay payload, picking the simple broadcast path for payloads that fit
/// [`SIMPLE_BROADCAST_MAX`] and the FEC path for the rest.
pub fn signed_broadcast(
    keypair: &AdnlKeyPair,
    payload: &[u8],
    flags: i32,
    date: i32,
) -> SignedBroadcast {
    if payload.len() <= SIMPLE_BROADCAST_MAX {
        SignedBroadcast::Simple(signed_simple_broadcast(
            keypair,
            payload.to_vec(),
            flags,
            date,
        ))
    } else {
        SignedBroadcast::Fec(fec_broadcast_parts(keypair, payload, flags, date))
    }
}

/// Generic signed broadcast handle over every connected peer.
///
/// The signed broadcast (or, for payloads above
/// [`SIMPLE_BROADCAST_MAX`], every signed RaptorQ source and repair
/// symbol, paced four parts per tick) is serialized once and fanned
/// out to all UDP and QUIC sessions in parallel, so a published
/// message leaves this node right away and keeps propagating through
/// upstream's neighbour re-broadcast.
#[must_use]
pub struct OverlayBroadcastPeer {
    manager: Arc<PeerManager>,
    keypair: AdnlKeyPair,
    quality: Option<Arc<PeerQualityRegistry>>,
}

impl OverlayBroadcastPeer {
    /// Creates a broadcast peer for the overlay `manager` carries
    /// sessions for, signing broadcasts with `keypair`.
    pub fn new(manager: Arc<PeerManager>, keypair: AdnlKeyPair) -> Self {
        Self {
            manager,
            keypair,
            quality: None,
        }
    }

    /// Fans every send out to sessions ordered by delivery quality: the
    /// highest-scoring peers are polled first, so a high-quality neighbour
    /// starts relaying the message while slower sessions are still being
    /// written. Without a registry the fan-out keeps insertion order.
    ///
    /// The node builder wires this with the client's quality registry.
    pub fn with_quality(mut self, quality: Arc<PeerQualityRegistry>) -> Self {
        self.quality = Some(quality);
        self
    }

    /// Signs `payload` and fans it out to every connected session.
    /// Returns the number of successful session sends.
    pub async fn broadcast(&self, payload: &[u8], flags: i32) -> Result<usize, OverlayError> {
        let manager = self.manager.clone();
        let keypair = self.keypair;
        let quality = self.quality.clone();
        // Ranked once per send and reused for every FEC part, so all
        // symbols of one broadcast leave in the same peer order.
        let ranked = match &quality {
            Some(registry) => registry.rank(&manager.peers().await),
            None => Vec::new(),
        };
        let date = now_i32();
        match signed_broadcast(&keypair, payload, flags, date) {
            SignedBroadcast::Simple(object) => {
                let sent = manager
                    .broadcast_object_in_order(Arc::from(object), &ranked)
                    .await;
                if sent == 0 {
                    return Err(OverlayError::Broadcast(
                        "no overlay sessions connected".to_owned(),
                    ));
                }
                log::debug!("overlay broadcast: simple sent to {sent} session(s)");
                Ok(sent)
            }
            SignedBroadcast::Fec(parts) => {
                let total = parts.len();
                let mut delivered = 0usize;
                let mut remaining = parts.into_iter();
                // Upstream paces FEC symbols at four per 10 ms tick
                // (`BroadcastFecActor::alarm`); the same cadence keeps
                // the fan-out from bursting `2 * symbols_count`
                // datagrams at every neighbour at once, and the first
                // four parts still leave immediately, so typical
                // externals (up to four symbols) finish in one round.
                loop {
                    let batch: Vec<_> = remaining.by_ref().take(FEC_PARTS_PER_TICK).collect();
                    if batch.is_empty() {
                        break;
                    }
                    let results = join_all(batch.into_iter().map(|part| {
                        // Wire ids `0..symbols_count` are
                        // source symbols, the dense tail above
                        // it is repair; `total` is always
                        // `2 * symbols_count`.
                        record_fec_part_sent(part.seqno >= total as i32 / 2);
                        let manager = manager.clone();
                        let ranked = ranked.clone();
                        let object = Arc::from(part.object);
                        async move { manager.broadcast_object_in_order(object, &ranked).await }
                    }))
                    .await;
                    delivered += results.into_iter().filter(|sent| *sent > 0).count();
                    if remaining.size_hint().0 == 0 {
                        break;
                    }
                    if delivered == 0 {
                        // No session took the first parts - there is
                        // nothing left to pace the rest into.
                        break;
                    }
                    tokio::time::sleep(FEC_SEND_TICK).await;
                }
                if delivered == 0 {
                    return Err(OverlayError::Broadcast(
                        "no overlay sessions connected".to_owned(),
                    ));
                }
                log::debug!(
                    "overlay broadcast: FEC sent {total} parts ({delivered} part fan-outs)"
                );
                Ok(delivered)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::overlay::{
        OverlayConfig, OverlaySession, OverlaySessionSender, PeerId, PeerQualityConfig,
        protocol_stats,
    };
    use futures::future::BoxFuture;

    use super::{
        AdnlKeyPair, Arc, Digest, FEC_SYMBOL_SIZE, FecType, Int256, OverlayBroadcast,
        OverlayBroadcastFec, OverlayBroadcastFecId, OverlayBroadcastFecPartId,
        OverlayBroadcastPeer, OverlayBroadcastToSign, PeerManager, PeerQualityRegistry,
        SIMPLE_BROADCAST_MAX, Sha256, SignedBroadcast, broadcast_src, fec_broadcast_parts,
        fec_wire_symbol_id_to_internal, now_i32, signed_broadcast, signed_simple_broadcast,
    };

    /// A BoC-shaped payload of `size` bytes: the BoC magic plus
    /// filler, long enough for the envelope checks to pass.
    fn test_boc(size: usize) -> Vec<u8> {
        let mut boc = vec![0xb5, 0xee, 0x9c, 0x72];
        boc.resize(size, 0x41);
        boc
    }

    #[test]
    fn simple_broadcast_verifies_through_payload_if_valid() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let date = now_i32();
        let at_cap = signed_broadcast(&keypair, &test_boc(SIMPLE_BROADCAST_MAX), 0, date);
        assert!(
            matches!(at_cap, SignedBroadcast::Simple(_)),
            "the cap-sized payload must stay a simple broadcast"
        );
        let over_cap = signed_broadcast(&keypair, &test_boc(SIMPLE_BROADCAST_MAX + 1), 0, date);
        assert!(
            matches!(over_cap, SignedBroadcast::Fec(_)),
            "one byte above the cap must switch to FEC"
        );
    }

    #[test]
    fn fec_parts_encode_decode_and_carry_valid_signatures() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let date = now_i32();
        let payload = test_boc(2000);
        let parts = fec_broadcast_parts(&keypair, &payload, 0, date);
        let expected_symbols = payload.len().div_ceil(FEC_SYMBOL_SIZE);
        assert_eq!(
            parts.len(),
            2 * expected_symbols,
            "every source symbol plus one repair symbol each"
        );

        let data_hash: [u8; 32] = Sha256::digest(&payload).into();
        let mut symbols = Vec::with_capacity(parts.len());
        for (index, part) in parts.iter().enumerate() {
            assert_eq!(part.seqno, index as i32, "parts are ordered by wire id");
            let object = tl_proto::deserialize::<OverlayBroadcastFec>(&part.object)
                .expect("valid FEC object");
            assert_eq!(
                object.data_hash.0, data_hash,
                "every part names the object hash"
            );
            assert_eq!(object.data_size as usize, payload.len());
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
                    assert_eq!(*data_size as usize, payload.len());
                    assert_eq!(*symbol_size as usize, FEC_SYMBOL_SIZE);
                    assert_eq!(
                        *symbols_count as usize, expected_symbols,
                        "fec.Type carries the source symbol count, not the part count"
                    );
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
            // This test checks the source-symbol fast path; repair
            // symbols get their own lossy reconstruction test below.
            if (object.seqno as usize) < expected_symbols {
                symbols.push((object.seqno as u32, object.data.clone()));
            }
        }

        // The source symbols must decode back to the payload, each
        // under its own symbol id.
        let mut decoder = raptorq::Decoder::new(raptorq::ObjectTransmissionInformation::new(
            payload.len() as u64,
            FEC_SYMBOL_SIZE as u16,
            1,
            1,
            1,
        ));
        for (symbol_id, symbol) in symbols {
            let packet =
                raptorq::EncodingPacket::new(raptorq::PayloadId::new(0, symbol_id), symbol);
            if let Some(reconstructed) = decoder.decode(packet) {
                assert_eq!(reconstructed, payload);
                return;
            }
        }
        panic!("source symbols must reconstruct the object");
    }

    #[test]
    fn fec_repair_symbols_recover_lost_source_parts() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let date = now_i32();
        let payload = test_boc(2000);
        let parts = fec_broadcast_parts(&keypair, &payload, 0, date);
        let expected_symbols = payload.len().div_ceil(FEC_SYMBOL_SIZE);
        assert!(expected_symbols > 1, "payload must span several symbols");
        let mut decoder = raptorq::Decoder::new(raptorq::ObjectTransmissionInformation::new(
            payload.len() as u64,
            FEC_SYMBOL_SIZE as u16,
            1,
            1,
            1,
        ));
        let mut reconstructed = None;
        for part in &parts {
            let object =
                tl_proto::deserialize::<OverlayBroadcastFec>(&part.object).expect("valid FEC");
            let wire = object.seqno as u32;
            // Simulate one lost source datagram: source symbol 1
            // never arrives, everything else does.
            if wire == 1 {
                continue;
            }
            let symbol_id = fec_wire_symbol_id_to_internal(wire, expected_symbols as u32);
            let packet = raptorq::EncodingPacket::new(
                raptorq::PayloadId::new(0, symbol_id),
                object.data.clone(),
            );
            if let Some(data) = decoder.decode(packet) {
                reconstructed = Some(data);
                break;
            }
        }
        assert_eq!(
            reconstructed.expect("repair symbols must cover the lost source part"),
            payload,
            "the object must reassemble without the lost source symbol"
        );
    }

    struct RecordingSession {
        peer: PeerId,
        log: Arc<tokio::sync::Mutex<Vec<PeerId>>>,
    }

    impl OverlaySession for RecordingSession {
        fn peer_id(&self) -> PeerId {
            self.peer
        }

        fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
            Box::pin(std::future::pending())
        }

        fn send(&mut self, _payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }

        fn send_object(&mut self, _object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
            let peer = self.peer;
            let log = Arc::clone(&self.log);
            Box::pin(async move {
                log.lock().await.push(peer);
                Ok(())
            })
        }

        fn sender(&self) -> Option<Arc<dyn OverlaySessionSender>> {
            Some(Arc::new(RecordingSender {
                peer: self.peer,
                log: Arc::clone(&self.log),
            }))
        }
    }

    /// The session's lock-free sender: it records into the same log, so
    /// fan-out tests observe sends without the parked receive loop's
    /// session lock ever entering the picture.
    struct RecordingSender {
        peer: PeerId,
        log: Arc<tokio::sync::Mutex<Vec<PeerId>>>,
    }

    impl OverlaySessionSender for RecordingSender {
        fn send(&self, _payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
            let peer = self.peer;
            let log = Arc::clone(&self.log);
            Box::pin(async move {
                log.lock().await.push(peer);
                Ok(())
            })
        }

        fn send_object(&self, _object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
            let peer = self.peer;
            let log = Arc::clone(&self.log);
            Box::pin(async move {
                log.lock().await.push(peer);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn quality_ranked_send_starts_best_peer_first() {
        let manager = Arc::new(
            PeerManager::new(OverlayConfig::default()).expect("peer manager must be created"),
        );
        let log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let best = PeerId::from_bytes([1; 32]);
        let rest = PeerId::from_bytes([2; 32]);
        for peer in [best, rest] {
            manager
                .add_session(Box::new(RecordingSession {
                    peer,
                    log: Arc::clone(&log),
                }))
                .await;
        }
        let registry = PeerQualityRegistry::new(PeerQualityConfig::default());
        registry.set_connected(vec![best, rest]);
        registry.record_first_delivery([7; 32], best, std::time::Instant::now());
        let broadcast = OverlayBroadcastPeer::new(
            Arc::clone(&manager),
            AdnlKeyPair::generate(&mut rand::rngs::OsRng),
        )
        .with_quality(Arc::new(registry));
        let raw = test_boc(64);
        broadcast
            .broadcast(&raw, 0)
            .await
            .expect("a connected peer set must accept the broadcast");
        assert_eq!(
            *log.lock().await,
            vec![best, rest],
            "the best-scoring peer must start its send first"
        );
        manager.shutdown_wait().await;
    }

    /// The paced FEC fan-out counts every source and repair part it
    /// hands to the sessions, so a live run can tell repair traffic
    /// leaving the node from source traffic.
    #[tokio::test]
    async fn fec_fanout_counts_source_and_repair_parts() {
        let manager = Arc::new(
            PeerManager::new(OverlayConfig::default()).expect("peer manager must be created"),
        );
        let log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        manager
            .add_session(Box::new(RecordingSession {
                peer: PeerId::from_bytes([3; 32]),
                log: Arc::clone(&log),
            }))
            .await;
        let broadcast = OverlayBroadcastPeer::new(
            Arc::clone(&manager),
            AdnlKeyPair::generate(&mut rand::rngs::OsRng),
        );
        let raw = test_boc(2048);
        let symbols_count = raw.len().div_ceil(FEC_SYMBOL_SIZE) as u64;
        assert!(symbols_count > 1, "the test payload must use the FEC path");
        let before = protocol_stats();
        broadcast
            .broadcast(&raw, 0)
            .await
            .expect("a connected peer set must accept the FEC broadcast"); // The counters are process-global and tests run in parallel, so
        // the deltas are lower bounds.
        let after = protocol_stats();
        assert!(
            after.fec_parts_sent >= before.fec_parts_sent + 2 * symbols_count,
            "every source and repair part must count as sent"
        );
        assert!(
            after.fec_repair_parts_sent >= before.fec_repair_parts_sent + symbols_count,
            "the repair half must count as repair parts"
        );
        manager.shutdown_wait().await;
    }
}
