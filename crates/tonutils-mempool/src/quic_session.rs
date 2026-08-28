#![allow(clippy::large_types_passed_by_value)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::future::join_all;
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};
use tl_proto::TlRead;
use tonutils_adnl::adnl::quic::QuicSession;
use tonutils_adnl::{AdnlAddress, KeyPair, PublicKey as AdnlPublicKey, now_i32};
use tonutils_overlay::{OverlayId, OverlaySession, PeerId, SeedDiscoveryLookup, SeedPeer};

/// QUIC port offset: upstream TON nodes listen for QUIC on `adnl_udp_port + 1000`.
const QUIC_PORT_OFFSET: u16 = 1000;
use tonutils_tl::tl::network::{
    Address, AddressListBoxed, DhtKey, DhtValueResult, OverlayBroadcast, OverlayBroadcastFec,
    OverlayMessage, OverlayNode, OverlayNodeToSign, OverlayNodesBoxed, PublicKey as TlPublicKey,
    QuicMessage, TonNodeExternalMessageBroadcast,
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
                if let Ok(data) = self.unwrap_overlay_payload(data_slice) {
                    return Ok(Arc::from(data));
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
            let remote_addr = SocketAddr::new(
                remote_addr.ip(),
                remote_addr.port().wrapping_add(QUIC_PORT_OFFSET),
            );
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

#[allow(dead_code)]
fn quic_dht_key_id(id: [u8; 32], name: &[u8]) -> tonutils_tl::Int256 {
    let dht_key = DhtKey {
        id: tonutils_tl::Int256(id),
        name: name.to_vec(),
        idx: 0,
    };
    // Hash the BOXED form (with constructor prefix) per upstream TON.
    tonutils_tl::Int256(Sha256::digest(dht_key.boxed_bytes()).into())
}

#[allow(dead_code)]
fn quic_valid_overlay_node(node: &OverlayNode, overlay: OverlayId, now: i32) -> bool {
    if node.overlay.0 != overlay.as_bytes() || node.version < now.saturating_sub(600) {
        return false;
    }
    let TlPublicKey::Ed25519 { key } = &node.id else {
        return false;
    };
    let Some(public_key) = AdnlPublicKey::from_bytes(key.0) else {
        return false;
    };
    let adnl_id = AdnlAddress::from(&public_key).to_bytes();
    let unsigned = OverlayNodeToSign {
        id: tonutils_tl::tl::network::AdnlIdShort {
            id: tonutils_tl::Int256(adnl_id),
        },
        overlay: node.overlay.clone(),
        version: node.version,
    };
    let signature = match node.signature.as_slice() {
        signature if signature.len() == 64 => signature,
        signature if signature.len() == 68 => &signature[4..],
        _ => return false,
    };
    let Ok(signature) = signature.try_into() else {
        return false;
    };
    public_key.verify_raw(&tl_proto::serialize(unsigned), &signature)
}

#[allow(dead_code)]
async fn quic_connect_to_seed(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: SocketAddr,
) -> Option<QuicSession> {
    let quic_addr = SocketAddr::new(address.ip(), address.port().wrapping_add(QUIC_PORT_OFFSET));
    match tokio::time::timeout(
        Duration::from_secs(5),
        QuicSession::connect(local_addr, quic_addr, local_keypair, remote),
    )
    .await
    {
        Err(_timeout) => {
            log::debug!("quic_connect_to_seed: connect to {quic_addr} timed out");
            None
        }
        Ok(Err(error)) => {
            log::debug!("quic_connect_to_seed: connect to {quic_addr} failed: {error}");
            None
        }
        Ok(Ok(session)) => Some(session),
    }
}

#[allow(dead_code)]
async fn quic_query_dht_value_seed(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: SocketAddr,
    key: tonutils_tl::Int256,
    count: usize,
    timeout: Duration,
) -> Option<DhtValueResult> {
    let session = quic_connect_to_seed(local_addr, local_keypair, remote, address).await?;
    match session
        .dht_find_value(key, count.min(i32::MAX as usize) as i32, timeout)
        .await
    {
        Ok(result) => Some(result),
        Err(error) => {
            log::debug!("quic_query_dht_value_seed: dht_find_value to {address} failed: {error}");
            None
        }
    }
}

#[allow(dead_code, clippy::too_many_arguments)]
async fn quic_query_overlay_seed(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: SocketAddr,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> Option<Vec<SeedPeer>> {
    let initial = SeedPeer {
        peer: PeerId::from_bytes(remote.to_bytes()),
        address: address.to_string(),
    };
    let overlay_dht_key = quic_dht_key_id(overlay_key, b"nodes");
    let per_query_timeout = timeout.min(Duration::from_secs(15));
    log::debug!(
        "quic_query_overlay_seed: seed={address} overlay_dht_key={}",
        overlay_dht_key.to_hex()
    );
    let mut frontier = vec![initial.clone()];
    let mut seen = std::collections::HashSet::new();
    let now = now_i32();

    for _ in 0..6 {
        let responses = join_all(frontier.drain(..).filter_map(|seed| {
            let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
            let address = seed.address.parse().ok()?;
            let overlay_dht_key = overlay_dht_key.clone();
            Some(async move {
                let response = quic_query_dht_value_seed(
                    local_addr,
                    local_keypair,
                    remote,
                    address,
                    overlay_dht_key,
                    max_records,
                    per_query_timeout,
                )
                .await?;
                Some((seed, response))
            })
        }))
        .await;
        let mut next = Vec::new();
        for response in responses.into_iter().flatten() {
            let (seed, response) = response;
            match response {
                DhtValueResult::Found { value } => {
                    let nodes: OverlayNodesBoxed = tl_proto::deserialize(&value.value).ok()?;
                    log::debug!(
                        "quic_query_overlay_seed: found {} overlay nodes from {address}",
                        nodes.nodes.len()
                    );
                    let mut result = Vec::new();
                    for node in nodes.nodes {
                        if !quic_valid_overlay_node(&node, overlay, now) {
                            continue;
                        }
                        if result.len() >= max_records {
                            continue;
                        }
                        let TlPublicKey::Ed25519 { key } = node.id else {
                            continue;
                        };
                        let overlay_public = AdnlPublicKey::from_bytes(key.0)?;
                        let address_key = quic_dht_key_id(
                            AdnlAddress::from(&overlay_public).to_bytes(),
                            b"address",
                        );
                        let Some(DhtValueResult::Found { value }) = quic_query_dht_value_seed(
                            local_addr,
                            local_keypair,
                            AdnlPublicKey::from_bytes(seed.peer.as_bytes())?,
                            seed.address.parse().ok()?,
                            address_key,
                            1,
                            per_query_timeout,
                        )
                        .await
                        else {
                            continue;
                        };
                        let address_list: AddressListBoxed =
                            tl_proto::deserialize(&value.value).ok()?;
                        let Some((peer, resolved_address)) =
                            address_list.addrs.into_iter().find_map(|address| {
                                let Address::Udp { ip, port } = address else {
                                    return None;
                                };
                                let port = u16::try_from(port).ok()?;
                                if port == 0 || ip == 0 {
                                    return None;
                                }
                                let TlPublicKey::Ed25519 { key } = &value.key.id else {
                                    return None;
                                };
                                Some((
                                    PeerId::from_bytes(key.0),
                                    format!(
                                        "{}:{port}",
                                        std::net::Ipv4Addr::from(ip.cast_unsigned())
                                    ),
                                ))
                            })
                        else {
                            continue;
                        };
                        if result
                            .iter()
                            .all(|candidate: &SeedPeer| candidate.peer != peer)
                        {
                            result.push(SeedPeer {
                                peer,
                                address: resolved_address,
                            });
                        }
                    }
                    if !result.is_empty() {
                        return Some(result);
                    }
                }
                DhtValueResult::NotFound { nodes } => {
                    log::debug!(
                        "quic_query_overlay_seed: not found, got {} closer nodes from {address}",
                        nodes.nodes.len()
                    );
                    for seed in
                        tonutils_overlay::select_typed_dht_peers(nodes.nodes, max_records, now)
                    {
                        if seen.insert((seed.peer, seed.address.clone())) {
                            next.push(seed);
                        }
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            log::debug!("quic_query_overlay_seed: frontier exhausted at {address}");
            break;
        }
    }

    log::debug!(
        "quic_query_overlay_seed: DHT lookup missed at {address}, trying overlay getRandomPeers"
    );
    let session = quic_connect_to_seed(local_addr, local_keypair, remote, address).await?;
    let overlay_int = tonutils_tl::Int256(overlay.as_bytes());
    let nodes = match session
        .overlay_get_random_peers(overlay_int, per_query_timeout)
        .await
    {
        Ok(nodes) => nodes,
        Err(error) => {
            log::debug!(
                "quic_query_overlay_seed: overlay_get_random_peers to {address} failed: {error}"
            );
            return None;
        }
    };
    let mut result = Vec::new();
    for node in nodes.nodes {
        if !quic_valid_overlay_node(&node, overlay, now) {
            continue;
        }
        let TlPublicKey::Ed25519 { key } = node.id else {
            continue;
        };
        let overlay_public = AdnlPublicKey::from_bytes(key.0)?;
        let adnl_id = AdnlAddress::from(&overlay_public).to_bytes();
        let address_key = quic_dht_key_id(adnl_id, b"address");
        let Some(DhtValueResult::Found { value }) = quic_query_dht_value_seed(
            local_addr,
            local_keypair,
            remote,
            address,
            address_key,
            1,
            per_query_timeout,
        )
        .await
        else {
            continue;
        };
        let address_list: AddressListBoxed = tl_proto::deserialize(&value.value).ok()?;
        let Some((peer, resolved_address)) = address_list.addrs.into_iter().find_map(|addr| {
            let Address::Udp { ip, port } = addr else {
                return None;
            };
            let port = u16::try_from(port).ok()?;
            if port == 0 || ip == 0 {
                return None;
            }
            let TlPublicKey::Ed25519 { key } = &value.key.id else {
                return None;
            };
            Some((
                PeerId::from_bytes(key.0),
                format!("{}:{port}", std::net::Ipv4Addr::from(ip.cast_unsigned())),
            ))
        }) else {
            continue;
        };
        if result
            .iter()
            .all(|candidate: &SeedPeer| candidate.peer != peer)
        {
            result.push(SeedPeer {
                peer,
                address: resolved_address,
            });
        }
    }
    if !result.is_empty() {
        return Some(result);
    }
    None
}

#[allow(dead_code)]
pub fn quic_overlay_lookup(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> SeedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let responses = join_all(seeds.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                Some(quic_query_overlay_seed(
                    local_addr,
                    local_keypair,
                    remote,
                    address,
                    overlay,
                    overlay_key,
                    max_records,
                    timeout,
                ))
            }))
            .await;
            let mut result = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for peers in responses.into_iter().flatten() {
                for peer in peers {
                    if seen.insert((peer.peer, peer.address.clone())) {
                        result.push(peer);
                    }
                }
            }
            log::debug!(
                "quic_overlay_lookup: returning {} peers from discovery",
                result.len()
            );
            result
        })
    })
}
