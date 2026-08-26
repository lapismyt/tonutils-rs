use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation, PayloadId};
use sha2::{Digest, Sha256};
use tl_proto::TlRead;
use tonutils_adnl::adnl::quic::QuicSession;
use tonutils_adnl::{KeyPair, now_i32};
use tonutils_overlay::{OverlayId, OverlaySession, PeerId, SeedPeer};
use tonutils_tl::tl::network::{
    OverlayBroadcast, OverlayBroadcastFec, OverlayMessage, QuicMessage,
    TonNodeExternalMessageBroadcast,
};

struct FecAssembly {
    decoder: Decoder,
    data_size: usize,
    symbol_size: i32,
    symbols_count: i32,
    last_seen: Instant,
}

pub struct QuicOverlaySession {
    peer: PeerId,
    session: Arc<QuicSession>,
    overlay: [u8; 32],
    fec: std::collections::HashMap<[u8; 32], FecAssembly>,
    last_keepalive: Instant,
}

impl QuicOverlaySession {
    pub fn new(session: Arc<QuicSession>, overlay: [u8; 32]) -> Self {
        let peer = PeerId::from_bytes(session.remote_public_key().to_bytes());
        Self {
            peer,
            session,
            overlay,
            fec: std::collections::HashMap::new(),
            last_keepalive: Instant::now(),
        }
    }

    fn unwrap_overlay_payload(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        if let Ok(broadcast) = tl_proto::deserialize::<TonNodeExternalMessageBroadcast>(data) {
            return Ok(broadcast.message.data);
        }
        if let Ok(fec) = tl_proto::deserialize::<OverlayBroadcastFec>(data) {
            let (fec_data_size, symbol_size, symbols_count) = match fec.fec {
                tonutils_tl::tl::network::FecType::RaptorQ {
                    data_size: fec_data_size,
                    symbol_size,
                    symbols_count,
                } => (fec_data_size, symbol_size, symbols_count),
                _ => return Err("unsupported overlay FEC type".to_owned()),
            };
            let data_size = fec.data_size;
            if data_size <= 0
                || data_size as usize > 1 << 20
                || symbol_size <= 0
                || symbols_count <= 0
                || fec_data_size != data_size
                || (data_size as usize).div_ceil(symbol_size as usize) != symbols_count as usize
                || fec.seqno < 0
            {
                return Err("invalid overlay FEC parameters".to_owned());
            }
            self.fec
                .retain(|_, state| state.last_seen.elapsed() < Duration::from_secs(90));
            if self.fec.len() >= 128 && !self.fec.contains_key(&fec.data_hash.0) {
                return Err("overlay FEC reassembly capacity exceeded".to_owned());
            }
            let state = self
                .fec
                .entry(fec.data_hash.0)
                .or_insert_with(|| FecAssembly {
                    decoder: Decoder::new(ObjectTransmissionInformation::new(
                        data_size as u64,
                        symbol_size as u16,
                        1,
                        1,
                        1,
                    )),
                    data_size: data_size as usize,
                    symbol_size,
                    symbols_count,
                    last_seen: Instant::now(),
                });
            if state.data_size != data_size as usize
                || state.symbol_size != symbol_size
                || state.symbols_count != symbols_count
            {
                return Err("overlay FEC metadata changed during reassembly".to_owned());
            }
            state.last_seen = Instant::now();
            let expected_size = state.data_size;
            let max_symbol_id = symbols_count as u32 + (symbols_count as u32 / 2) + 1024;
            if fec.seqno as u32 > max_symbol_id {
                return Err("overlay FEC packet has an excessive symbol id".to_owned());
            }
            let packet = EncodingPacket::new(PayloadId::new(0, fec.seqno as u32), fec.data);
            let Some(reconstructed) = state.decoder.decode(packet) else {
                return Err("overlay FEC payload is incomplete".to_owned());
            };
            self.fec.remove(&fec.data_hash.0);
            let hash: [u8; 32] = Sha256::digest(&reconstructed).into();
            if hash != fec.data_hash.0 || reconstructed.len() != expected_size {
                return Err("overlay FEC reconstructed data mismatch".to_owned());
            }
            let broadcast = tl_proto::deserialize::<TonNodeExternalMessageBroadcast>(
                &reconstructed,
            )
            .map_err(|_| "overlay FEC reconstructed payload is not external message".to_owned())?;
            return Ok(broadcast.message.data);
        }
        match tl_proto::deserialize::<OverlayBroadcast>(data) {
            Ok(OverlayBroadcast::Unicast { data }) => Ok(data),
            Ok(broadcast) => broadcast
                .payload_if_valid(now_i32())
                .map(ToOwned::to_owned)
                .ok_or_else(|| "invalid overlay broadcast".to_owned()),
            Err(_) => Err("invalid overlay payload".to_owned()),
        }
    }
}

impl OverlaySession for QuicOverlaySession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        Box::pin(async move {
            loop {
                if self.last_keepalive.elapsed() >= Duration::from_secs(5) {
                    let _ = self
                        .session
                        .overlay_get_random_peers(
                            tonutils_tl::Int256(self.overlay),
                            Duration::from_secs(3),
                        )
                        .await;
                    self.last_keepalive = Instant::now();
                }
                let stream = tokio::select! {
                    result = self.session.connection().accept_bi() => {
                        match result {
                            Ok(stream) => stream,
                            Err(e) => {
                                return Err(format!("QUIC accept_bi failed: {e}"));
                            }
                        }
                    }
                    () = tokio::time::sleep(Duration::from_secs(1)) => {
                        continue;
                    }
                };
                let (mut send, mut recv) = stream;
                let buf = match tokio::time::timeout(
                    Duration::from_secs(5),
                    recv.read_to_end(1024 * 1024),
                )
                .await
                {
                    Ok(Ok(buf)) => buf,
                    _ => {
                        let _ = send.finish();
                        let _ = send.write_all(&[]).await;
                        continue;
                    }
                };
                let _ = send.finish();
                if buf.is_empty() {
                    continue;
                }
                if buf.len() >= 4 {
                    let id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                    if id == 0x6d2960d1
                        && let Ok(message) = tl_proto::deserialize::<QuicMessage>(&buf)
                    {
                        let data = message.data;
                        let mut data_slice = data.as_slice();
                        let message_overlay = match OverlayMessage::read_from(&mut data_slice) {
                            Ok(OverlayMessage::Message { overlay })
                            | Ok(OverlayMessage::MessageWithExtra { overlay, .. }) => overlay,
                            _ => continue,
                        };
                        if message_overlay.0 != self.overlay {
                            continue;
                        }
                        if let Ok(data) = self.unwrap_overlay_payload(data_slice) {
                            return Ok(Arc::from(data));
                        }
                    }
                }
                let mut data_slice = buf.as_slice();
                let message_overlay = match OverlayMessage::read_from(&mut data_slice) {
                    Ok(OverlayMessage::Message { overlay })
                    | Ok(OverlayMessage::MessageWithExtra { overlay, .. }) => overlay,
                    _ => continue,
                };
                if message_overlay.0 != self.overlay {
                    continue;
                }
                match self.unwrap_overlay_payload(data_slice) {
                    Ok(data) => return Ok(Arc::from(data)),
                    Err(_) => {}
                }
            }
        })
    }

    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let mut data = tl_proto::serialize(OverlayBroadcast::Unicast {
                data: payload.to_vec(),
            });
            let mut wrapped = Vec::with_capacity(4 + 32 + 4 + data.len());
            wrapped.extend_from_slice(&0x75252420u32.to_le_bytes());
            wrapped.extend_from_slice(&self.overlay);
            let message = OverlayMessage::Message {
                overlay: tonutils_tl::Int256(self.overlay),
            };
            let mut message_bytes = tl_proto::serialize(message);
            message_bytes.append(&mut data);
            wrapped.extend_from_slice(&message_bytes);
            self.session
                .send_message(wrapped)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// Creates an `OverlaySessionFactory` that connects to peers via QUIC.
pub fn quic_overlay_factory(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
) -> crate::OverlaySessionFactory {
    Arc::new(move |seed: SeedPeer| {
        let local_addr = local_addr;
        let local_keypair = local_keypair;
        let overlay = overlay;
        Box::pin(async move {
            let remote_key = tonutils_adnl::PublicKey::from_bytes(seed.peer.as_bytes())
                .ok_or_else(|| format!("invalid peer public key for {:?}", seed.peer))?;
            let remote_addr: SocketAddr = seed
                .address
                .parse()
                .map_err(|e| format!("invalid seed address {}: {e}", seed.address))?;
            let session = QuicSession::connect(local_addr, remote_addr, local_keypair, remote_key)
                .await
                .map_err(|e| format!("QUIC connect failed: {e}"))?;
            let session = Arc::new(session);
            let _ = session
                .overlay_get_random_peers(
                    tonutils_tl::Int256(overlay.as_bytes()),
                    Duration::from_secs(3),
                )
                .await;
            Ok(
                Box::new(QuicOverlaySession::new(session, overlay.as_bytes()))
                    as Box<dyn OverlaySession>,
            )
        })
    })
}
