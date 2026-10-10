use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::future::join_all;
use raptorq::{Decoder, EncodingPacket, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Instant;
use tl_proto::TlRead;
use tonutils_adnl::{
    AdnlAddress, AdnlUdpSession, AdnlUdpTransport, KeyPair, PublicKey as AdnlPublicKey, now_i32,
};
use tonutils_overlay::{
    OverlayId, OverlaySession, PeerId, SeedDiscoveryLookup, SeedPeer, TypedDiscoveryLookup,
};
use tonutils_tl::Message as AdnlMessage;
use tonutils_tl::tl::network::{
    Address, AddressListBoxed, DhtKey, DhtValueResult, OverlayBroadcast, OverlayBroadcastFec,
    OverlayMessage, OverlayNode, OverlayNodeToSign, OverlayNodesBoxed, PacketContents,
    PublicKey as TlPublicKey, TonNodeExternalMessageBroadcast,
};

use crate::overlay_inbound::OverlayMemberCache;
use crate::{overlay_inbound, protocol_stats};

/// Interval between `overlay.getRandomPeers` keepalives on a live session.
///
/// Every keepalive refreshes the `version` field of this node's signed
/// record, so it has to stay well inside the upstream `overlay_peer_ttl` of
/// 600 seconds.  Ten seconds is what pytoniq uses for the same purpose; a one
/// second cadence produced tens of thousands of queries per run for no extra
/// admission chance, because the peer's pending-peer drain - not our query
/// rate - decides when a node is pinged.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Adapter exposing an authenticated direct ADNL UDP session to the overlay.
pub struct AdnlUdpOverlaySession {
    peer: PeerId,
    session: AdnlUdpSession,
    overlay: Option<OverlayId>,
    fec: HashMap<[u8; 32], FecAssembly>,
    last_keepalive: Instant,
    /// Last datagram accepted from the peer, used by the pool idle deadline.
    last_activity: Instant,
    /// Members this session may gossip in `overlay.getRandomPeers` answers.
    ///
    /// Sessions created by one factory share a single cache, see
    /// [`overlay_factory`](crate::overlay_factory).
    members: OverlayMemberCache,
}

mod lookup;
mod publish;

pub use lookup::*;
pub use publish::{DhtAddressPublisher, PUBLISH_INTERVAL, address_publisher};

struct FecAssembly {
    decoder: Decoder,
    data_size: usize,
    symbol_size: i32,
    symbols_count: i32,
    last_seen: Instant,
}

/// Checks an `overlay.node` record against `overlay` and the current clock.
///
/// `pub(crate)` so [`crate::overlay_inbound`] can re-validate members before
/// gossiping them back.
pub(crate) fn valid_overlay_node(node: &OverlayNode, overlay: OverlayId, now: i32) -> bool {
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

impl AdnlUdpOverlaySession {
    /// Returns the shared-transport session for one peer.
    ///
    /// Every ADNL/UDP session of this scanner - live overlay
    /// members, DHT seed queries, address resolution, and peer
    /// growth alike - resolves its socket through
    /// [`AdnlUdpTransport::for_node`], so all of them send from
    /// one source address per node id, exactly the upstream
    /// `AdnlNetworkManager` model.  A peer therefore keeps a
    /// single, always-reachable address for this node instead of
    /// the dead port a one-shot lookup socket would otherwise
    /// leave in its connection table.
    #[allow(clippy::large_types_passed_by_value)]
    async fn session_on_transport(
        local_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        remote_addr: std::net::SocketAddr,
        remote_public: AdnlPublicKey,
    ) -> Result<AdnlUdpSession, String> {
        let transport = AdnlUdpTransport::for_node(local_addr, local_keypair)
            .await
            .map_err(|error| error.to_string())?;
        Ok(transport.session_for_peer(remote_public, remote_addr))
    }

    pub async fn connect(
        peer: PeerId,
        local_addr: std::net::SocketAddr,
        remote_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        remote_public: AdnlPublicKey,
    ) -> Result<Self, String> {
        let session =
            Self::session_on_transport(local_addr, local_keypair, remote_addr, remote_public)
                .await?;
        Ok(Self {
            peer,
            session,
            overlay: None,
            fec: HashMap::new(),
            last_keepalive: Instant::now(),
            last_activity: Instant::now(),
            members: OverlayMemberCache::default(),
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
        let session =
            Self::session_on_transport(local_addr, local_keypair, remote_addr, remote_public)
                .await?;
        session
            .establish_channel(timeout)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self {
            peer,
            session,
            overlay: None,
            fec: HashMap::new(),
            last_keepalive: Instant::now(),
            last_activity: Instant::now(),
            members: OverlayMemberCache::default(),
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
        log::debug!(
            "overlay UDP session established: peer={peer:?} address={remote_addr} overlay={overlay}"
        );
        log::debug!("overlay UDP session sending overlay.getRandomPeers: peer={peer:?}");
        protocol_stats::record_random_peers_query_sent();
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
        let session =
            Self::session_on_transport(local_addr, local_keypair, remote_addr, remote_public)
                .await?;
        session
            .establish_channel(timeout)
            .await
            .map_err(|error| error.to_string())?;
        let session = Self {
            peer,
            session,
            overlay: Some(overlay),
            fec: HashMap::new(),
            last_keepalive: Instant::now(),
            last_activity: Instant::now(),
            members: OverlayMemberCache::default(),
        };
        if let Err(error) = session
            .session
            .overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()), timeout)
            .await
        {
            log::warn!("overlay handshake skipped for {peer:?}: {error}");
        }
        protocol_stats::record_random_peers_query_sent();
        Ok(session)
    }
}

impl OverlaySession for AdnlUdpOverlaySession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn last_activity(&self) -> Option<std::time::Instant> {
        Some(self.last_activity)
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        Box::pin(async move {
            loop {
                // Refreshing this node's signed record keeps its `version`
                // inside the peer's `overlay_peer_ttl` window, which is what
                // lets `add_peer` keep the node eligible for the peer's
                // pending-peer drain and eventually an `overlay.ping`.
                if self.last_keepalive.elapsed() >= KEEPALIVE_INTERVAL
                    && let Some(overlay) = self.overlay
                {
                    let _ = self
                        .session
                        .send_overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()))
                        .await;
                    protocol_stats::record_random_peers_query_sent();
                    self.last_keepalive = Instant::now();
                }
                let packet = match tokio::time::timeout(
                    Duration::from_secs(1),
                    self.session.recv_contents(),
                )
                .await
                {
                    Ok(packet) => packet.map_err(|error| error.to_string())?,
                    Err(_) => {
                        log::trace!("overlay UDP receive timeout: peer={:?}", self.peer);
                        continue;
                    }
                };
                self.last_activity = Instant::now();
                let messages = packet
                    .message
                    .into_iter()
                    .chain(packet.messages.into_iter().flatten());
                let mut channel_changed = false;
                for message in messages {
                    log::trace!(
                        "overlay UDP packet classified: peer={:?} message={}",
                        self.peer,
                        match &message {
                            AdnlMessage::Query { .. } => "query",
                            AdnlMessage::Answer { .. } => "answer",
                            AdnlMessage::Custom { .. } => "custom",
                            AdnlMessage::CreateChannel { .. } => "create-channel",
                            AdnlMessage::ConfirmChannel { .. } => "confirm-channel",
                            _ => "control",
                        }
                    );
                    if let AdnlMessage::Answer { answer, .. } = &message {
                        overlay_inbound::trace_membership(
                            &self.peer,
                            &self.session,
                            self.overlay,
                            &self.members,
                            answer,
                        );
                        continue;
                    }
                    if let AdnlMessage::Query { query_id, query } = &message
                        && self.answer_overlay_query(query_id, query).await
                    {
                        continue;
                    }
                    if matches!(
                        message,
                        AdnlMessage::CreateChannel { .. } | AdnlMessage::ConfirmChannel { .. }
                    ) {
                        channel_changed = true;
                    }
                    if let AdnlMessage::Custom { data } = message {
                        // Custom messages are the only overlay traffic that
                        // can become a `MempoolEvent`, so a run that never
                        // logs this line proves no broadcast reached the node
                        // rather than that one arrived and was mis-parsed.
                        log::debug!(
                            "overlay UDP custom message: peer={:?} len={} prefix={:02x?}",
                            self.peer,
                            data.len(),
                            &data[..data.len().min(8)]
                        );
                        let data = if let Some(overlay) = self.overlay {
                            let mut data = data.as_slice();
                            let message_overlay = match OverlayMessage::read_from(&mut data) {
                                Ok(OverlayMessage::Message { overlay })
                                | Ok(OverlayMessage::MessageWithExtra { overlay, .. }) => overlay,
                                _ => {
                                    log::trace!(
                                        "overlay UDP custom payload has no overlay header: peer={:?}",
                                        self.peer
                                    );
                                    continue;
                                }
                            };
                            if message_overlay.0 != overlay.as_bytes() {
                                log::trace!(
                                    "overlay UDP custom payload for another overlay: peer={:?} got={:02x?} joined={:02x?}",
                                    self.peer,
                                    message_overlay.0,
                                    overlay.as_bytes(),
                                );
                                continue;
                            }
                            match self.unwrap_overlay_payload(data) {
                                Ok(data) => data,
                                Err(error) => {
                                    log::trace!(
                                        "overlay UDP custom payload rejected: peer={:?} reason={error}",
                                        self.peer
                                    );
                                    continue;
                                }
                            }
                        } else {
                            match self.unwrap_overlay_payload(&data) {
                                Ok(data) => data,
                                Err(error) => {
                                    log::trace!(
                                        "direct UDP payload rejected: peer={:?} reason={error}",
                                        self.peer
                                    );
                                    continue;
                                }
                            }
                        };
                        protocol_stats::record_custom_message();
                        return Ok(Arc::from(data));
                    }
                }
                if channel_changed && let Some(overlay) = self.overlay {
                    log::debug!(
                        "overlay UDP channel changed; resubscribing with overlay.getRandomPeers: peer={:?}",
                        self.peer
                    );
                    let _ = self
                        .session
                        .send_overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()))
                        .await;
                    protocol_stats::record_random_peers_query_sent();
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
    /// Answers the overlay queries this node is expected to serve.
    ///
    /// The dispatch, including the `overlay.query` wrapper that upstream
    /// always adds, lives in [`crate::overlay_inbound`].
    ///
    /// Returns `true` when the query was recognised and answered.
    async fn answer_overlay_query(&mut self, query_id: &tonutils_tl::Int256, query: &[u8]) -> bool {
        let Some(answer) = overlay_inbound::build_overlay_answer(
            &self.peer,
            &self.session,
            self.overlay,
            &self.members,
            query,
        ) else {
            return false;
        };
        if let Err(error) = self.session.send_answer(query_id.clone(), answer).await {
            protocol_stats::record_query_answer_failed();
            log::trace!(
                "overlay query answer failed: peer={:?} error={error}",
                self.peer
            );
        }
        true
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
            if fec.data.len() < 4 {
                return Err("overlay FEC packet is truncated".to_owned());
            }
            let packet = EncodingPacket::deserialize(&fec.data);
            if packet.data().len() != symbol_size as usize {
                return Err("overlay FEC symbol has invalid length".to_owned());
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

#[cfg(test)]
mod tests;
