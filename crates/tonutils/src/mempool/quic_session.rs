use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    adnl::{AdnlAddress, AdnlKeyPair, AdnlPublicKey, now_i32, quic::QuicSession},
    mempool::{
        OverlayMemberEndpoints, OverlaySessionFactory, QuicEndpointSource,
        broadcast::{MAX_FEC_SOURCE_SYMBOLS, fec_wire_symbol_id_to_internal},
        known_member_seeds, member_quic_target,
        packet_dedup::PacketPreDedup,
        protocol_stats::{
            record_fec_decoded, record_fec_param_rejection, record_fec_symbol,
            record_payload_duplicate,
        },
        record_member,
        udp_session::parse_address_record,
        unwrap_external_message_payload,
    },
    overlay::{
        OverlayId, OverlaySession, OverlaySessionSender, PeerId, SeedDiscoveryLookup, SeedPeer,
        select_typed_dht_peers,
    },
    tl::{
        Int256,
        network::{
            AdnlIdShort, DhtKey, DhtValueResult, FecType, OverlayBroadcast, OverlayBroadcastFec,
            OverlayMessage, OverlayNode, OverlayNodeToSign, OverlayNodesBoxed, QuicMessage,
            TlPublicKey, TonNodeExternalMessageBroadcast,
        },
    },
};

use futures::future::{BoxFuture, join_all};
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};
use tl_proto::TlRead;

/// QUIC port offset: upstream TON nodes listen for QUIC on `adnl_udp_port + 1000`.
const QUIC_PORT_OFFSET: u16 = 1000;

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
    /// Raw-payload pre-dedup shared with every session of one factory,
    /// so byte-identical flood copies stop before the TL parse and
    /// signature verification.
    pre_dedup: PacketPreDedup,
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
            pre_dedup: PacketPreDedup::default(),
        }
    }

    fn unwrap_overlay_payload(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        if let Ok(broadcast) = tl_proto::deserialize::<TonNodeExternalMessageBroadcast>(data) {
            return Ok(broadcast.message.data);
        }
        if let Ok(fec) = tl_proto::deserialize::<OverlayBroadcastFec>(data) {
            let (fec_data_size, symbol_size, symbols_count) = match fec.fec {
                FecType::RaptorQ {
                    data_size: fec_data_size,
                    symbol_size,
                    symbols_count,
                } => (fec_data_size, symbol_size, symbols_count),
                _ => {
                    record_fec_param_rejection();
                    return Err("unsupported overlay FEC type".to_owned());
                }
            };
            let data_size = fec.data_size;
            // `symbol_size` and `symbols_count` also bound what the
            // raptorq transmission information is built from: its
            // constructor asserts the RFC 6330 `K'_max` symbol bound
            // (56 403), and the decoder config truncates `symbol_size`
            // to `u16` - crafted values would otherwise panic the
            // receive loop.
            if data_size <= 0
                || data_size as usize > 1 << 20
                || symbol_size <= 0
                || symbols_count <= 0
                || symbol_size > u16::MAX as i32
                || symbols_count as usize > MAX_FEC_SOURCE_SYMBOLS
                || fec_data_size != data_size
                || (data_size as usize).div_ceil(symbol_size as usize) != symbols_count as usize
                || fec.seqno < 0
            {
                record_fec_param_rejection();
                return Err("invalid overlay FEC parameters".to_owned());
            }
            self.fec
                .retain(|_, state| state.last_seen.elapsed() < Duration::from_secs(90));
            if self.fec.len() >= 128 && !self.fec.contains_key(&fec.data_hash.0) {
                return Err("overlay FEC reassembly capacity exceeded".to_owned());
            }
            if fec.data.len() < 4 {
                return Err("overlay FEC packet is truncated".to_owned());
            }
            // Upstream's `BroadcastsFec::process_broadcast`
            // (`overlay/broadcast-fec.cpp`) carries the raw RaptorQ
            // symbol bytes in `data` and the symbol id in `seqno`;
            // this crate's own FEC senders used the raptorq crate's
            // serialized `EncodingPacket` (payload id plus symbol
            // bytes).  Both are accepted: a `data` field exactly
            // `symbol_size` bytes long is the raw upstream symbol,
            // anything longer must be a serialized packet whose
            // payload decodes to `symbol_size` bytes.
            let (wire_seqno, symbol) = if fec.data.len() == symbol_size as usize {
                (fec.seqno as u32, fec.data.clone())
            } else {
                let packet = EncodingPacket::deserialize(&fec.data);
                if packet.data().len() != symbol_size as usize {
                    return Err("overlay FEC symbol has invalid length".to_owned());
                }
                if packet.payload_id().source_block_number() != 0 {
                    return Err("overlay FEC packet has invalid source block".to_owned());
                }
                (
                    packet.payload_id().encoding_symbol_id(),
                    packet.data().to_vec(),
                )
            };
            // The wire `seqno` numbers source symbols
            // `0..symbols_count` and repair symbols densely above it
            // (`BroadcastFecActor` emits `(size / 768 + 1) * 2` of
            // them).  The raptorq decoder instead reserves
            // `[symbols_count, K')` for the zero-padded extended
            // symbols and numbers repair symbols from `K'`, so repair
            // ids shift up by `K' - symbols_count` - upstream's
            // `td::fec::raptorq::Decoder::add_symbol` applies the same
            // shift.  The bound check runs on the wire id, before the
            // shift.
            let max_symbol_id = symbols_count as u32 + (symbols_count as u32 / 2) + 1024;
            if wire_seqno > max_symbol_id {
                record_fec_param_rejection();
                return Err("overlay FEC packet has an excessive symbol id".to_owned());
            }
            record_fec_symbol(wire_seqno >= symbols_count as u32);
            let packet = EncodingPacket::new(
                raptorq::PayloadId::new(
                    0,
                    fec_wire_symbol_id_to_internal(wire_seqno, symbols_count as u32),
                ),
                symbol,
            );
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
            record_fec_decoded();
            return Ok(broadcast.message.data);
        }
        match tl_proto::deserialize::<OverlayBroadcast>(data) {
            Ok(OverlayBroadcast::Unicast { data }) => Ok(unwrap_external_message_payload(data)),
            Ok(broadcast) => broadcast
                .payload_if_valid(now_i32())
                // A verified simple broadcast still carries the
                // `tonNode.externalMessageBroadcast` wrapper, so the
                // deliverable BoC is one unwrap below the payload
                // (`unwrap_external_message_payload`).
                .map(|payload| unwrap_external_message_payload(payload.to_owned()))
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
                        .overlay_get_random_peers(Int256(self.overlay), Duration::from_secs(3))
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
                    if id == 0xe003df31
                        && let Ok(message) = tl_proto::deserialize::<QuicMessage>(&buf)
                    {
                        let data = message.data;
                        let mut data_slice = data.as_slice();
                        let message_overlay = match OverlayMessage::read_from(&mut data_slice) {
                            Ok(
                                OverlayMessage::Message { overlay }
                                | OverlayMessage::MessageWithExtra { overlay, .. },
                            ) => overlay,
                            _ => continue,
                        };
                        if message_overlay.0 != self.overlay {
                            continue;
                        }
                        if self.pre_dedup.is_duplicate(data_slice) {
                            record_payload_duplicate();
                            continue;
                        }
                        if let Ok(data) = self.unwrap_overlay_payload(data_slice) {
                            return Ok(Arc::from(data));
                        }
                    }
                }
                let mut data_slice = buf.as_slice();
                let message_overlay = match OverlayMessage::read_from(&mut data_slice) {
                    Ok(
                        OverlayMessage::Message { overlay }
                        | OverlayMessage::MessageWithExtra { overlay, .. },
                    ) => overlay,
                    _ => continue,
                };
                if message_overlay.0 != self.overlay {
                    continue;
                }
                if self.pre_dedup.is_duplicate(data_slice) {
                    record_payload_duplicate();
                    continue;
                }
                if let Ok(data) = self.unwrap_overlay_payload(data_slice) {
                    return Ok(Arc::from(data));
                }
            }
        })
    }

    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        let sender = self.sender_handle();
        Box::pin(async move { sender.send_payload(payload).await })
    }

    fn send_object(&mut self, object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        let sender = self.sender_handle();
        Box::pin(async move { sender.send_raw_object(object).await })
    }

    fn sender(&self) -> Option<Arc<dyn OverlaySessionSender>> {
        Some(Arc::new(self.sender_handle()))
    }
}

/// Lock-free send handle over the internally synchronized QUIC session:
/// the connection lives behind a shared `Arc`, so a sender clone
/// frames and sends without the receive loop's session lock.
struct QuicOverlaySender {
    session: Arc<QuicSession>,
    overlay: [u8; 32],
}

impl QuicOverlaySender {
    async fn send_payload(&self, payload: Arc<[u8]>) -> Result<(), String> {
        let mut wrapped = Vec::with_capacity(36 + 4 + payload.len());
        wrapped.extend_from_slice(&0x75252420u32.to_le_bytes());
        wrapped.extend_from_slice(&self.overlay);
        wrapped.extend_from_slice(&tl_proto::serialize(OverlayBroadcast::Unicast {
            data: payload.to_vec(),
        }));
        self.session
            .send_message(wrapped)
            .await
            .map_err(|error| error.to_string())
    }

    async fn send_raw_object(&self, object: Arc<[u8]>) -> Result<(), String> {
        let mut wrapped = Vec::with_capacity(36 + object.len());
        wrapped.extend_from_slice(&0x75252420u32.to_le_bytes());
        wrapped.extend_from_slice(&self.overlay);
        wrapped.extend_from_slice(&object);
        self.session
            .send_message(wrapped)
            .await
            .map_err(|error| error.to_string())
    }
}

impl OverlaySessionSender for QuicOverlaySender {
    fn send(&self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(self.send_payload(payload))
    }

    fn send_object(&self, object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(self.send_raw_object(object))
    }
}

impl QuicOverlaySession {
    /// A cheap sender view onto the same connection.
    fn sender_handle(&self) -> QuicOverlaySender {
        QuicOverlaySender {
            session: Arc::clone(&self.session),
            overlay: self.overlay,
        }
    }
}

/// Creates an `OverlaySessionFactory` that connects to peers via QUIC.
///
/// Each peer's QUIC endpoint is derived as `adnl_udp_port +
/// QUIC_PORT_OFFSET`.  Prefer [`quic_overlay_factory_with_endpoints`] when
/// overlay members are known.
pub fn quic_overlay_factory(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
) -> OverlaySessionFactory {
    quic_overlay_factory_with_endpoints(
        local_addr,
        local_keypair,
        overlay,
        OverlayMemberEndpoints::default(),
    )
}

/// [`quic_overlay_factory`] that prefers a peer's advertised
/// `adnl.address.quic` endpoint from `endpoints`, falling back to
/// `adnl_udp_port + QUIC_PORT_OFFSET` - the same preference order upstream
/// `QuicSender::get_ip_address` applies.
pub fn quic_overlay_factory_with_endpoints(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    endpoints: OverlayMemberEndpoints,
) -> OverlaySessionFactory {
    // One pre-dedup for every session this factory spawns: flood copies
    // of one broadcast arrive from different peers, and the first
    // sighting of a payload hash suppresses the rest.
    let pre_dedup = PacketPreDedup::default();
    Arc::new(move |seed: SeedPeer| {
        let local_addr = local_addr;
        let local_keypair = local_keypair;
        let overlay = overlay;
        let endpoints = Arc::clone(&endpoints);
        let pre_dedup = pre_dedup.clone();
        Box::pin(async move {
            let remote_key = AdnlPublicKey::from_bytes(seed.peer.as_bytes())
                .ok_or_else(|| format!("invalid peer public key for {:?}", seed.peer))?;
            let remote_addr: SocketAddr = seed
                .address
                .parse()
                .map_err(|e| format!("invalid seed address {}: {e}", seed.address))?;
            let (remote_addr, source) = quic_target_for(&endpoints, seed.peer, remote_addr);
            log::debug!(
                "quic probe: member={} target={remote_addr} source={} via=session",
                hex::encode(seed.peer.as_bytes()),
                source.label()
            );
            let session = QuicSession::connect(local_addr, remote_addr, local_keypair, remote_key)
                .await
                .map_err(|e| format!("QUIC connect failed: {e}"))?;
            let session = Arc::new(session);
            let _ = session
                .overlay_get_random_peers(Int256(overlay.as_bytes()), Duration::from_secs(3))
                .await;
            let mut overlay_session = QuicOverlaySession::new(session, overlay.as_bytes());
            overlay_session.pre_dedup = pre_dedup;
            Ok(Box::new(overlay_session) as Box<dyn OverlaySession>)
        })
    })
}

fn quic_dht_key_id(id: [u8; 32], name: &[u8]) -> Int256 {
    let dht_key = DhtKey {
        id: Int256(id),
        name: name.to_vec(),
        idx: 0,
    };
    // Hash the BOXED form (with constructor prefix) per upstream TON.
    Int256(Sha256::digest(dht_key.boxed_bytes()).into())
}

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
        id: AdnlIdShort {
            id: Int256(adnl_id),
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

/// Dials a probe target, treating silence as a miss rather than an error:
/// upstream full nodes listen on `adnl_udp_port + 1000`, but nothing listens
/// on that port of a `dht-server`, so a timeout is the common case for
/// non-member targets.
///
/// Logs one line per probe in the form
/// `quic probe: member=<hex> target=<ip:port> source=<advertised|udp+1000>
/// result=<connected|timeout|error: ...> via=<context>`.
async fn quic_probe_connect(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    remote: AdnlPublicKey,
    address: SocketAddr,
    endpoints: &OverlayMemberEndpoints,
    context: &str,
) -> Option<QuicSession> {
    let peer = PeerId::from_bytes(remote.to_bytes());
    let (quic_addr, source) = quic_target_for(endpoints, peer, address);
    let attempt = tokio::time::timeout(
        Duration::from_secs(5),
        QuicSession::connect(local_addr, quic_addr, local_keypair, remote),
    )
    .await;
    let (session, result) = match attempt {
        Err(_) => (None, "timeout".to_owned()),
        Ok(Err(error)) => (None, format!("error: {error}")),
        Ok(Ok(session)) => (Some(session), "connected".to_owned()),
    };
    log::debug!(
        "quic probe: member={} target={quic_addr} source={} result={result} via={context}",
        hex::encode(peer.as_bytes()),
        source.label()
    );
    session
}

/// Resolves the QUIC endpoint to dial for a peer: an advertised
/// `adnl.address.quic` entry wins, otherwise `udp + QUIC_PORT_OFFSET`.
fn quic_target_for(
    endpoints: &OverlayMemberEndpoints,
    peer: PeerId,
    udp: SocketAddr,
) -> (SocketAddr, QuicEndpointSource) {
    member_quic_target(endpoints, peer, udp, QUIC_PORT_OFFSET)
}

#[allow(clippy::too_many_arguments)]
async fn quic_query_dht_value_seed(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    remote: AdnlPublicKey,
    address: SocketAddr,
    key: Int256,
    count: usize,
    timeout: Duration,
    endpoints: &OverlayMemberEndpoints,
) -> Option<DhtValueResult> {
    let session = quic_probe_connect(
        local_addr,
        local_keypair,
        remote,
        address,
        endpoints,
        "dht.findValue",
    )
    .await?;
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

#[allow(clippy::too_many_arguments)]
async fn quic_query_overlay_seed(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    remote: AdnlPublicKey,
    address: SocketAddr,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
    endpoints: &OverlayMemberEndpoints,
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
                    endpoints,
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
                            endpoints,
                        )
                        .await
                        else {
                            continue;
                        };
                        let Some(resolved) =
                            parse_address_record(value, seed.address.parse().ok()?, endpoints)
                        else {
                            continue;
                        };
                        if result
                            .iter()
                            .all(|candidate: &SeedPeer| candidate.peer != resolved.peer)
                        {
                            result.push(resolved);
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
                    for seed in select_typed_dht_peers(nodes.nodes, max_records, now) {
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
        "quic_query_overlay_seed: DHT lookup missed at {address}, trying overlay.getRandomPeers"
    );
    let session = quic_probe_connect(
        local_addr,
        local_keypair,
        remote,
        address,
        endpoints,
        "getRandomPeers",
    )
    .await?;
    let overlay_int = Int256(overlay.as_bytes());
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
    let mut valid_members = 0usize;
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
        valid_members += 1;
        let Some(DhtValueResult::Found { value }) = quic_query_dht_value_seed(
            local_addr,
            local_keypair,
            remote,
            address,
            address_key,
            1,
            per_query_timeout,
            endpoints,
        )
        .await
        else {
            continue;
        };
        let Some(resolved) = parse_address_record(value, address, endpoints) else {
            continue;
        };
        if result
            .iter()
            .all(|candidate: &SeedPeer| candidate.peer != resolved.peer)
        {
            result.push(resolved);
        }
    }
    // Membership evidence, mirroring the UDP growth path: a peer that answers
    // `overlay.getRandomPeers` for this overlay with valid signed node records
    // is an overlay member, so its endpoint belongs in the QUIC probe registry.
    if valid_members > 0 {
        record_member(endpoints, PeerId::from_bytes(remote.to_bytes()), address);
    }
    if !result.is_empty() {
        return Some(result);
    }
    None
}

/// DHT-backed overlay seed discovery over QUIC, mirroring
/// [`crate::udp_overlay_lookup`] for the QUIC session factory.
///
/// Probes `seeds`, deriving each QUIC endpoint as
/// `adnl_udp_port + QUIC_PORT_OFFSET`.  Prefer
/// [`quic_overlay_lookup_with_endpoints`] whenever overlay members are known:
/// the config seeds are `dht-server` deployments, which have no QUIC
/// listener, so probing them can only time out.
pub fn quic_overlay_lookup(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> SeedDiscoveryLookup {
    quic_overlay_lookup_with_endpoints(
        local_addr,
        local_keypair,
        overlay,
        overlay_key,
        max_records,
        timeout,
        OverlayMemberEndpoints::default(),
    )
}

/// [`quic_overlay_lookup`] that probes the overlay members registered in
/// `endpoints` instead of the bootstrap seeds.
///
/// Upstream runs QUIC in `validator-engine` (validators and full nodes),
/// never in `dht-server`, and members already resolved by the UDP discovery
/// path are exactly that node set, so registered members are the only
/// targets worth dialling.  Bootstrap seeds are used solely as an
/// empty-registry fallback, which is logged.  Probes are capped at
/// `max_records` members so a full 256-entry registry cannot fan out 256
/// concurrent QUIC connections.
pub fn quic_overlay_lookup_with_endpoints(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
    endpoints: OverlayMemberEndpoints,
) -> SeedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        let endpoints = Arc::clone(&endpoints);
        Box::pin(async move {
            // Registered members first.  Sorted by peer id so the probed
            // subset of a large registry is stable across runs.
            let mut members: Vec<(PeerId, SocketAddr)> = known_member_seeds(&endpoints);
            members.sort_by_key(|(peer, ..)| peer.as_bytes());
            let mut targets: Vec<SeedPeer> = members
                .into_iter()
                .take(max_records.max(1))
                .map(|(peer, address)| SeedPeer {
                    peer,
                    address: address.to_string(),
                })
                .collect();
            if targets.is_empty() {
                log::debug!(
                    "quic_overlay_lookup: no overlay members registered, falling back to {} bootstrap seed(s) - dht-server deployments have no QUIC listener, so probes are expected to time out",
                    seeds.len()
                );
                targets = seeds;
            } else {
                log::debug!(
                    "quic_overlay_lookup: probing {} registered overlay member(s)",
                    targets.len()
                );
            }
            let responses = join_all(targets.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                let endpoints = Arc::clone(&endpoints);
                Some(async move {
                    quic_query_overlay_seed(
                        local_addr,
                        local_keypair,
                        remote,
                        address,
                        overlay,
                        overlay_key,
                        max_records,
                        timeout,
                        &endpoints,
                    )
                    .await
                })
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

#[cfg(test)]
mod tests {
    use crate::adnl::QuicServer;

    use super::*;

    /// Spawns a client `QuicSession` towards a loopback QUIC
    /// server.  The test accepts the connection - which completes
    /// the handshake - and then reads the streams the session
    /// sends through it.
    async fn quic_loopback_client(
        overlay: OverlayId,
    ) -> (tokio::task::JoinHandle<QuicSession>, QuicServer, OverlayId) {
        let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
        let server_addr = server.local_addr().unwrap();
        let client = tokio::spawn(async move {
            QuicSession::connect(
                "127.0.0.1:0".parse().unwrap(),
                server_addr,
                client_keypair,
                server_keypair.public_key,
            )
            .await
            .unwrap()
        });
        (client, server, overlay)
    }

    #[tokio::test]
    async fn send_frames_one_unicast_after_the_overlay_header() {
        let overlay = OverlayId::from_name(b"quic-framing");
        let (client, server, overlay) = quic_loopback_client(overlay).await;
        let (connection, _sni) = server.accept().await.unwrap();
        let mut session =
            QuicOverlaySession::new(Arc::new(client.await.unwrap()), overlay.as_bytes());
        let reader = tokio::spawn(async move {
            let (_send, mut recv) = connection.accept_bi().await.unwrap();
            let buf = recv.read_to_end(1024 * 1024).await.unwrap();
            tl_proto::deserialize::<QuicMessage>(&buf).unwrap().data
        });
        let payload: Arc<[u8]> = Arc::from(vec![1u8, 2, 3]);
        session.send(payload.clone()).await.unwrap();
        let data = reader.await.unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&0x75252420u32.to_le_bytes());
        expected.extend_from_slice(&overlay.as_bytes());
        expected.extend_from_slice(&tl_proto::serialize(OverlayBroadcast::Unicast {
            data: payload.to_vec(),
        }));
        assert_eq!(
            data, expected,
            "send must frame exactly one overlay.unicast after the overlay header"
        );
    }

    #[tokio::test]
    async fn send_object_frames_the_raw_object_after_the_overlay_header() {
        let overlay = OverlayId::from_name(b"quic-framing");
        let (client, server, overlay) = quic_loopback_client(overlay).await;
        let (connection, _sni) = server.accept().await.unwrap();
        let mut session =
            QuicOverlaySession::new(Arc::new(client.await.unwrap()), overlay.as_bytes());
        let reader = tokio::spawn(async move {
            let (_send, mut recv) = connection.accept_bi().await.unwrap();
            let buf = recv.read_to_end(1024 * 1024).await.unwrap();
            tl_proto::deserialize::<QuicMessage>(&buf).unwrap().data
        });
        let object: Arc<[u8]> = Arc::from(vec![7u8, 8, 9]);
        session.send_object(object.clone()).await.unwrap();
        let data = reader.await.unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&0x75252420u32.to_le_bytes());
        expected.extend_from_slice(&overlay.as_bytes());
        expected.extend_from_slice(&object);
        assert_eq!(
            data, expected,
            "send_object must frame the raw object after the overlay header"
        );
    }
}
