use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::future::join_all;
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;
use tonutils_adnl::{AdnlUdpSession, KeyPair, PublicKey as AdnlPublicKey};
use tonutils_overlay::{OverlayId, OverlaySession, PeerId, SeedPeer, TypedDiscoveryLookup};
use tonutils_tl::Message as AdnlMessage;
use tonutils_tl::tl::network::{
    OverlayBroadcast, OverlayBroadcastFec, PacketContents, TonNodeExternalMessageBroadcast,
};

/// Adapter exposing an authenticated direct ADNL UDP session to the overlay.
pub struct AdnlUdpOverlaySession {
    peer: PeerId,
    session: AdnlUdpSession,
    overlay: Option<OverlayId>,
    fec: HashMap<[u8; 32], FecAssembly>,
}

struct FecAssembly {
    decoder: Decoder,
    data_size: usize,
    symbol_size: i32,
    symbols_count: i32,
    last_seen: Instant,
}

pub fn udp_dht_lookup(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    node_count: i32,
    timeout: Duration,
) -> TypedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let responses = join_all(seeds.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                Some(async move {
                    let mut session =
                        AdnlUdpSession::connect(local_addr, address, local_keypair, remote)
                            .await
                            .ok()?;
                    session
                        .dht_find_node(tonutils_tl::Int256::random(), node_count, timeout)
                        .await
                        .ok()
                        .map(|nodes| nodes.nodes)
                })
            }))
            .await;
            responses.into_iter().flatten().flatten().collect()
        })
    })
}

pub fn udp_iterative_dht_lookup(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    node_count: i32,
    rounds: usize,
    timeout: Duration,
) -> TypedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let mut frontier = seeds;
            let mut discovered = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .min(i32::MAX as u64) as i32;
            for _ in 0..rounds.max(1) {
                let responses = join_all(frontier.into_iter().filter_map(|seed| {
                    let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                    let address = seed.address.parse().ok()?;
                    Some(query_dht_seed(
                        local_addr,
                        local_keypair,
                        remote,
                        address,
                        node_count,
                        timeout,
                    ))
                }))
                .await;
                frontier = Vec::new();
                for nodes in responses.into_iter().flatten() {
                    for node in nodes {
                        let key = match &node.id {
                            tonutils_tl::tl::network::PublicKey::Ed25519 { key } => key.0,
                            _ => continue,
                        };
                        if seen.insert(key) {
                            frontier.extend(tonutils_overlay::select_typed_dht_peers(
                                [node.clone()],
                                8,
                                now,
                            ));
                            discovered.push(node);
                        }
                    }
                }
                if frontier.is_empty() {
                    break;
                }
            }
            discovered
        })
    })
}

#[allow(clippy::large_types_passed_by_value)]
async fn query_dht_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    node_count: i32,
    timeout: Duration,
) -> Option<Vec<tonutils_tl::tl::network::DhtNode>> {
    let mut session = AdnlUdpSession::connect(local_addr, address, local_keypair, remote)
        .await
        .ok()?;
    session
        .dht_find_node(tonutils_tl::Int256::random(), node_count, timeout)
        .await
        .ok()
        .map(|nodes| nodes.nodes)
}

pub fn direct_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
) -> crate::OverlaySessionFactory {
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        Box::pin(async move {
            let remote = remote.ok_or_else(|| "seed peer is not a valid Ed25519 key".to_owned())?;
            Ok(Box::new(
                AdnlUdpOverlaySession::connect(
                    seed.peer,
                    local_addr,
                    seed.address
                        .parse()
                        .map_err(|error| format!("invalid seed address: {error}"))?,
                    local_keypair,
                    remote,
                )
                .await?,
            ) as Box<dyn OverlaySession>)
        })
    })
}

pub fn channel_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    timeout: Duration,
) -> crate::OverlaySessionFactory {
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        Box::pin(async move {
            let remote = remote.ok_or_else(|| "seed peer is not a valid Ed25519 key".to_owned())?;
            Ok(Box::new(
                AdnlUdpOverlaySession::connect_with_channel(
                    seed.peer,
                    local_addr,
                    seed.address
                        .parse()
                        .map_err(|error| format!("invalid seed address: {error}"))?,
                    local_keypair,
                    remote,
                    timeout,
                )
                .await?,
            ) as Box<dyn OverlaySession>)
        })
    })
}

pub fn overlay_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    channel_timeout: Option<Duration>,
) -> crate::OverlaySessionFactory {
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        Box::pin(async move {
            let remote = remote.ok_or_else(|| "seed peer is not a valid Ed25519 key".to_owned())?;
            let session = match channel_timeout {
                Some(timeout) => {
                    AdnlUdpOverlaySession::connect_for_overlay_with_channel(
                        seed.peer,
                        overlay,
                        local_addr,
                        seed.address
                            .parse()
                            .map_err(|error| format!("invalid seed address: {error}"))?,
                        local_keypair,
                        remote,
                        timeout,
                    )
                    .await?
                }
                None => {
                    AdnlUdpOverlaySession::connect_for_overlay(
                        seed.peer,
                        overlay,
                        local_addr,
                        seed.address
                            .parse()
                            .map_err(|error| format!("invalid seed address: {error}"))?,
                        local_keypair,
                        remote,
                    )
                    .await?
                }
            };
            Ok(Box::new(session) as Box<dyn OverlaySession>)
        })
    })
}

impl AdnlUdpOverlaySession {
    pub async fn connect(
        peer: PeerId,
        local_addr: std::net::SocketAddr,
        remote_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        remote_public: AdnlPublicKey,
    ) -> Result<Self, String> {
        let session =
            AdnlUdpSession::connect(local_addr, remote_addr, local_keypair, remote_public)
                .await
                .map_err(|error| error.to_string())?;
        Ok(Self {
            peer,
            session,
            overlay: None,
            fec: HashMap::new(),
        })
    }

    pub async fn connect_with_channel(
        peer: PeerId,
        local_addr: std::net::SocketAddr,
        remote_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        remote_public: AdnlPublicKey,
        timeout: Duration,
    ) -> Result<Self, String> {
        let session = AdnlUdpSession::connect_with_channel(
            local_addr,
            remote_addr,
            local_keypair,
            remote_public,
            timeout,
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(Self {
            peer,
            session,
            overlay: None,
            fec: HashMap::new(),
        })
    }

    pub async fn connect_for_overlay(
        peer: PeerId,
        overlay: OverlayId,
        local_addr: std::net::SocketAddr,
        remote_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        remote_public: AdnlPublicKey,
    ) -> Result<Self, String> {
        let mut session =
            Self::connect(peer, local_addr, remote_addr, local_keypair, remote_public).await?;
        session.overlay = Some(overlay);
        Ok(session)
    }

    pub async fn connect_for_overlay_with_channel(
        peer: PeerId,
        overlay: OverlayId,
        local_addr: std::net::SocketAddr,
        remote_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        remote_public: AdnlPublicKey,
        timeout: Duration,
    ) -> Result<Self, String> {
        let session = AdnlUdpSession::connect_with_channel(
            local_addr,
            remote_addr,
            local_keypair,
            remote_public,
            timeout,
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(Self {
            peer,
            session,
            overlay: Some(overlay),
            fec: HashMap::new(),
        })
    }
}

impl OverlaySession for AdnlUdpOverlaySession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        Box::pin(async move {
            loop {
                let packet = self
                    .session
                    .recv_contents()
                    .await
                    .map_err(|error| error.to_string())?;
                let messages = packet
                    .message
                    .into_iter()
                    .chain(packet.messages.into_iter().flatten());
                for message in messages {
                    if let AdnlMessage::Custom { data } = message {
                        let data = if let Some(overlay) = self.overlay {
                            if data.len() < 36 || data[..4] != 0x75252420u32.to_le_bytes() {
                                return Err("missing overlay message prefix".to_owned());
                            }
                            if data[4..36] != overlay.as_bytes() {
                                return Err("overlay id mismatch".to_owned());
                            }
                            match self.unwrap_overlay_payload(&data[36..]) {
                                Ok(data) => data,
                                Err(_) => continue,
                            }
                        } else {
                            match self.unwrap_overlay_payload(&data) {
                                Ok(data) => data,
                                Err(_) => continue,
                            }
                        };
                        return Ok(Arc::from(data));
                    }
                }
            }
        })
    }

    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let mut data = tl_proto::serialize(OverlayBroadcast::Unicast {
                data: payload.to_vec(),
            });
            if let Some(overlay) = self.overlay {
                let mut wrapped = Vec::with_capacity(36 + data.len());
                wrapped.extend_from_slice(&0x75252420u32.to_le_bytes());
                wrapped.extend_from_slice(&overlay.as_bytes());
                wrapped.append(&mut data);
                data = wrapped;
            }
            self.session
                .send_contents(PacketContents {
                    rand1: vec![0; 7],
                    flags: (),
                    from: None,
                    from_short: None,
                    message: Some(AdnlMessage::Custom { data }),
                    messages: None,
                    address: None,
                    priority_address: None,
                    seqno: None,
                    confirm_seqno: None,
                    recv_addr_list_version: None,
                    recv_priority_addr_list_version: None,
                    reinit_date: None,
                    dst_reinit_date: None,
                    signature: None,
                    rand2: vec![0; 7],
                })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }
}

impl AdnlUdpOverlaySession {
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
                || fec.data.len() < 4
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
            let packet = EncodingPacket::deserialize(&fec.data);
            if packet.payload_id().source_block_number() != 0 {
                return Err("overlay FEC packet has invalid source block".to_owned());
            }
            let max_symbol_id = symbols_count as u32 + (symbols_count as u32 / 2) + 1024;
            if packet.payload_id().encoding_symbol_id() > max_symbol_id {
                return Err("overlay FEC packet has an excessive symbol id".to_owned());
            }
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
                .payload_if_valid(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .min(i32::MAX as u64) as i32,
                )
                .map(ToOwned::to_owned)
                .ok_or_else(|| "invalid overlay broadcast".to_owned()),
            Err(_) => Err("invalid overlay payload".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raptorq::Encoder;
    use tonutils_adnl::KeyPair;
    use tonutils_tl::tl::network::{TonNodeExternalMessage, TonNodeExternalMessageBroadcast};

    #[tokio::test]
    async fn reassembles_single_source_raptorq_external_message() {
        let external = tl_proto::serialize(TonNodeExternalMessageBroadcast {
            message: TonNodeExternalMessage {
                data: vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3],
            },
        });
        let encoder = Encoder::with_defaults(&external, 128);
        let config = encoder.get_config();
        let packet = encoder
            .get_encoded_packets(0)
            .into_iter()
            .next()
            .expect("encoder must produce a source packet");
        let hash: [u8; 32] = Sha256::digest(&external).into();
        let local = KeyPair::generate(&mut rand::rngs::OsRng);
        let remote = KeyPair::generate(&mut rand::rngs::OsRng);
        let local_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let remote_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let session = AdnlUdpSession::connect(local_addr, remote_addr, local, remote.public_key)
            .await
            .unwrap();
        let mut adapter = AdnlUdpOverlaySession {
            peer: PeerId::from_bytes([1; 32]),
            session,
            overlay: None,
            fec: HashMap::new(),
        };
        let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
        let fec = OverlayBroadcastFec {
            src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![1] },
            certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
            data_hash: tonutils_tl::Int256(hash),
            data_size: external.len() as i32,
            flags: 0,
            data: packet.serialize(),
            seqno: 0,
            fec: tonutils_tl::tl::network::FecType::RaptorQ {
                data_size: external.len() as i32,
                symbol_size: config.symbol_size() as i32,
                symbols_count,
            },
            date: 0,
            signature: Vec::new(),
        };
        let payload = adapter
            .unwrap_overlay_payload(&tl_proto::serialize(fec))
            .unwrap();
        assert_eq!(payload, vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3]);
    }

    #[tokio::test]
    async fn waits_for_all_source_symbols_before_publishing_fec_payload() {
        let external = tl_proto::serialize(TonNodeExternalMessageBroadcast {
            message: TonNodeExternalMessage { data: vec![7; 300] },
        });
        let encoder = Encoder::with_defaults(&external, 64);
        let config = encoder.get_config();
        let packets = encoder.get_encoded_packets(0);
        assert!(packets.len() > 1);
        let hash: [u8; 32] = Sha256::digest(&external).into();
        let local = KeyPair::generate(&mut rand::rngs::OsRng);
        let remote = KeyPair::generate(&mut rand::rngs::OsRng);
        let local_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let remote_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let session = AdnlUdpSession::connect(local_addr, remote_addr, local, remote.public_key)
            .await
            .unwrap();
        let mut adapter = AdnlUdpOverlaySession {
            peer: PeerId::from_bytes([2; 32]),
            session,
            overlay: None,
            fec: HashMap::new(),
        };
        let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
        for (index, packet) in packets.into_iter().enumerate() {
            let fec = OverlayBroadcastFec {
                src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![2] },
                certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
                data_hash: tonutils_tl::Int256(hash),
                data_size: external.len() as i32,
                flags: 0,
                data: packet.serialize(),
                seqno: index as i32,
                fec: tonutils_tl::tl::network::FecType::RaptorQ {
                    data_size: external.len() as i32,
                    symbol_size: config.symbol_size() as i32,
                    symbols_count,
                },
                date: 0,
                signature: Vec::new(),
            };
            let result = adapter.unwrap_overlay_payload(&tl_proto::serialize(fec));
            if index + 1 == symbols_count as usize {
                assert_eq!(result.unwrap(), vec![7; 300]);
            } else {
                assert!(result.is_err());
            }
        }
    }
}
