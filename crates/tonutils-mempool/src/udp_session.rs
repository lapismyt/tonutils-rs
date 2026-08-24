use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::future::join_all;
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation, PayloadId};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;
use tl_proto::TlRead;
use tonutils_adnl::{AdnlAddress, AdnlUdpSession, KeyPair, PublicKey as AdnlPublicKey};
use tonutils_overlay::{
    OverlayId, OverlaySession, PeerId, SeedDiscoveryLookup, SeedPeer, TypedDiscoveryLookup,
};
use tonutils_tl::Message as AdnlMessage;
use tonutils_tl::tl::network::{
    Address, AddressListBoxed, DhtKey, DhtValueResult, OverlayBroadcast, OverlayBroadcastFec,
    OverlayMessage, OverlayNode, OverlayNodeToSign, OverlayNodesBoxed, OverlayQuery,
    PacketContents, PublicKey as TlPublicKey, TonNodeExternalMessageBroadcast,
};

/// Adapter exposing an authenticated direct ADNL UDP session to the overlay.
pub struct AdnlUdpOverlaySession {
    peer: PeerId,
    session: AdnlUdpSession,
    overlay: Option<OverlayId>,
    fec: HashMap<[u8; 32], FecAssembly>,
    last_keepalive: Instant,
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

pub fn udp_overlay_lookup(
    local_addr: std::net::SocketAddr,
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
                Some(query_overlay_seed(
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
                        eprintln!(
                            "udp_overlay_lookup: discovered peer {} at {}",
                            hex::encode(peer.peer.as_bytes()),
                            peer.address
                        );
                        result.push(peer);
                        if result.len() >= max_records {
                            return result;
                        }
                    }
                }
            }
            eprintln!(
                "udp_overlay_lookup: returning {} peers from discovery",
                result.len()
            );
            result
        })
    })
}

#[allow(clippy::large_types_passed_by_value, clippy::too_many_arguments)]
async fn query_overlay_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> Option<Vec<SeedPeer>> {
    let initial = SeedPeer {
        peer: PeerId::from_bytes(remote.to_bytes()),
        address: address.to_string(),
    };
    let overlay_dht_key = dht_key_id(overlay_key, b"nodes");
    let per_query_timeout = timeout.min(Duration::from_secs(5));
    log::debug!(
        "query_overlay_seed: seed={address} overlay_dht_key={}",
        overlay_dht_key.to_hex()
    );
    eprintln!(
        "query_overlay_seed: seed={address} overlay_dht_key={}",
        overlay_dht_key.to_hex()
    );
    let mut frontier = vec![initial];
    let mut seen = std::collections::HashSet::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i32::MAX as u64) as i32;

    for _ in 0..6 {
        let responses = join_all(frontier.drain(..).filter_map(|seed| {
            let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
            let address = seed.address.parse().ok()?;
            let overlay_dht_key = overlay_dht_key.clone();
            Some(async move {
                let response = query_dht_value_seed(
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
                        "query_overlay_seed: found {} overlay nodes from {address}",
                        nodes.nodes.len()
                    );
                    let mut result = Vec::new();
                    for node in nodes.nodes {
                        if !valid_overlay_node(&node, overlay, now) {
                            log::debug!(
                                "query_overlay_seed: skipping node (overlay mismatch or expired)"
                            );
                            continue;
                        }
                        if result.len() >= max_records {
                            continue;
                        }
                        let TlPublicKey::Ed25519 { key } = node.id else {
                            continue;
                        };
                        let overlay_public = AdnlPublicKey::from_bytes(key.0)?;
                        let address_key =
                            dht_key_id(AdnlAddress::from(&overlay_public).to_bytes(), b"address");
                        let Some(DhtValueResult::Found { value }) = query_dht_value_seed(
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
                        let Some((peer, address)) =
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
                            result.push(SeedPeer { peer, address });
                        }
                    }
                    if !result.is_empty() {
                        return Some(result);
                    }
                }
                DhtValueResult::NotFound { nodes } => {
                    log::debug!(
                        "query_overlay_seed: not found, got {} closer nodes from {address}",
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
            log::debug!("query_overlay_seed: frontier exhausted at {address}");
            break;
        }
    }
    log::debug!("query_overlay_seed: returning None for {address}");
    eprintln!("query_overlay_seed: returning None for {address}");
    None
}

#[allow(clippy::large_types_passed_by_value)]
async fn query_dht_value_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    key: tonutils_tl::Int256,
    count: usize,
    timeout: Duration,
) -> Option<DhtValueResult> {
    let mut session =
        match AdnlUdpSession::connect(local_addr, address, local_keypair, remote).await {
            Ok(session) => session,
            Err(error) => {
                eprintln!("query_dht_value_seed: connect to {address} failed: {error}");
                return None;
            }
        };
    match session
        .dht_find_value(key, count.min(i32::MAX as usize) as i32, timeout)
        .await
    {
        Ok(result) => Some(result),
        Err(error) => {
            eprintln!("query_dht_value_seed: dht_find_value to {address} failed: {error}");
            None
        }
    }
}

fn valid_overlay_node(node: &OverlayNode, overlay: OverlayId, now: i32) -> bool {
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

fn dht_key_id(id: [u8; 32], name: &[u8]) -> tonutils_tl::Int256 {
    tonutils_tl::Int256(
        Sha256::digest(tl_proto::serialize(DhtKey {
            id: tonutils_tl::Int256(id),
            name: name.to_vec(),
            idx: 0,
        }))
        .into(),
    )
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
            last_keepalive: Instant::now(),
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
            last_keepalive: Instant::now(),
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
        session
            .session
            .send_overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()))
            .await
            .map_err(|error| error.to_string())?;
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
        let mut session = Self {
            peer,
            session,
            overlay: Some(overlay),
            fec: HashMap::new(),
            last_keepalive: Instant::now(),
        };
        if let Err(error) = session
            .session
            .overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()), timeout)
            .await
        {
            log::debug!("overlay handshake skipped for {peer:?}: {error}");
        }
        Ok(session)
    }
}

impl OverlaySession for AdnlUdpOverlaySession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        Box::pin(async move {
            loop {
                if self.last_keepalive.elapsed() >= Duration::from_secs(5)
                    && let Some(overlay) = self.overlay
                {
                    let _ = self
                        .session
                        .send_overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()))
                        .await;
                    self.last_keepalive = Instant::now();
                }
                let packet = match tokio::time::timeout(
                    Duration::from_secs(5),
                    self.session.recv_contents(),
                )
                .await
                {
                    Ok(packet) => packet.map_err(|error| error.to_string())?,
                    Err(_) => continue,
                };
                let messages = packet
                    .message
                    .into_iter()
                    .chain(packet.messages.into_iter().flatten());
                let mut channel_changed = false;
                for message in messages {
                    if let AdnlMessage::Query { query_id, query } = &message
                        && let Ok(OverlayQuery::Ping) =
                            OverlayQuery::read_from(&mut query.as_slice())
                        && self.overlay.is_some()
                    {
                        let _ = self
                            .session
                            .send_answer(query_id.clone(), tl_proto::serialize(OverlayQuery::Ping))
                            .await;
                        continue;
                    }
                    if matches!(
                        message,
                        AdnlMessage::CreateChannel { .. } | AdnlMessage::ConfirmChannel { .. }
                    ) {
                        channel_changed = true;
                    }
                    if let AdnlMessage::Custom { data } = message {
                        let data = if let Some(overlay) = self.overlay {
                            let mut data = data.as_slice();
                            let message_overlay = match OverlayMessage::read_from(&mut data) {
                                Ok(OverlayMessage::Message { overlay })
                                | Ok(OverlayMessage::MessageWithExtra { overlay, .. }) => overlay,
                                _ => continue,
                            };
                            if message_overlay.0 != overlay.as_bytes() {
                                continue;
                            }
                            match self.unwrap_overlay_payload(data) {
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
                if channel_changed && let Some(overlay) = self.overlay {
                    let _ = self
                        .session
                        .send_overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()))
                        .await;
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
            last_keepalive: Instant::now(),
        };
        let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
        let fec = OverlayBroadcastFec {
            src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![1] },
            certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
            data_hash: tonutils_tl::Int256(hash),
            data_size: external.len() as i32,
            flags: 0,
            data: packet.split().1,
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
            last_keepalive: Instant::now(),
        };
        let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
        for (index, packet) in packets.into_iter().enumerate() {
            let fec = OverlayBroadcastFec {
                src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![2] },
                certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
                data_hash: tonutils_tl::Int256(hash),
                data_size: external.len() as i32,
                flags: 0,
                data: packet.split().1,
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

    #[tokio::test]
    async fn ignores_wrong_overlay_packets_without_killing_session() {
        let client_key = KeyPair::generate(&mut rand::rngs::OsRng);
        let server_key = KeyPair::generate(&mut rand::rngs::OsRng);
        let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let server_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_addr = server_socket.local_addr().unwrap();
        let overlay = OverlayId::from_name(b"expected-overlay");
        let mut receiver = AdnlUdpOverlaySession::connect_for_overlay(
            PeerId::from_bytes(server_key.public_key.to_bytes()),
            overlay,
            client_addr,
            server_addr,
            client_key,
            server_key.public_key,
        )
        .await
        .unwrap();
        drop(server_socket);
        let mut sender =
            AdnlUdpSession::connect(server_addr, client_addr, server_key, client_key.public_key)
                .await
                .unwrap();

        let mut wrong_overlay = Vec::new();
        wrong_overlay.extend_from_slice(&0x75252420u32.to_le_bytes());
        wrong_overlay.extend_from_slice(&OverlayId::from_name(b"wrong-overlay").as_bytes());
        wrong_overlay.extend_from_slice(&[1, 2, 3]);
        let packet = |data| PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Custom { data }),
            messages: None,
            address: None,
            priority_address: None,
            recv_addr_list_version: None,
            recv_priority_addr_list_version: None,
            reinit_date: None,
            dst_reinit_date: None,
            signature: None,
            rand2: vec![0; 7],
            seqno: None,
            confirm_seqno: None,
        };
        sender.send_contents(packet(wrong_overlay)).await.unwrap();

        let mut valid_overlay = Vec::new();
        valid_overlay.extend_from_slice(&0x75252420u32.to_le_bytes());
        valid_overlay.extend_from_slice(&overlay.as_bytes());
        valid_overlay.extend(tl_proto::serialize(TonNodeExternalMessageBroadcast {
            message: TonNodeExternalMessage {
                data: vec![9, 8, 7],
            },
        }));
        sender.send_contents(packet(valid_overlay)).await.unwrap();

        let payload = tokio::time::timeout(Duration::from_secs(1), receiver.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(payload.as_ref(), [9, 8, 7]);
    }

    #[test]
    fn validates_overlay_node_signature_and_timestamp_window() {
        let key = KeyPair::generate(&mut rand::rngs::OsRng);
        let overlay = OverlayId::from_name(b"overlay");
        let now = 1_000;
        let version = now + 60;
        let public_key = tonutils_tl::tl::network::PublicKey::Ed25519 {
            key: tonutils_tl::Int256(key.public_key.to_bytes()),
        };
        let adnl_id = tonutils_adnl::AdnlAddress::from(&key.public_key).to_bytes();
        let signature = key.sign_raw(&tl_proto::serialize(OverlayNodeToSign {
            id: tonutils_tl::tl::network::AdnlIdShort {
                id: tonutils_tl::Int256(adnl_id),
            },
            overlay: tonutils_tl::Int256(overlay.as_bytes()),
            version,
        }));
        let node = OverlayNode {
            id: public_key,
            overlay: tonutils_tl::Int256(overlay.as_bytes()),
            version,
            signature: signature.to_vec(),
        };
        assert!(valid_overlay_node(&node, overlay, now));
        let mut prefixed = vec![0xff, 0xff, 0xff, 0xff];
        prefixed.extend_from_slice(&signature);
        let prefixed_node = OverlayNode {
            signature: prefixed,
            ..node.clone()
        };
        assert!(valid_overlay_node(&prefixed_node, overlay, now));
        assert!(!valid_overlay_node(&node, overlay, now + 1_100));
    }
}
