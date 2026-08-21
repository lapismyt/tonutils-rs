use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::future::join_all;
use tonutils_adnl::{AdnlUdpSession, KeyPair, PublicKey as AdnlPublicKey};
use tonutils_overlay::{OverlayId, OverlaySession, PeerId, SeedPeer, TypedDiscoveryLookup};
use tonutils_tl::Message as AdnlMessage;
use tonutils_tl::tl::network::{OverlayBroadcast, PacketContents};

/// Adapter exposing an authenticated direct ADNL UDP session to the overlay.
pub struct AdnlUdpOverlaySession {
    peer: PeerId,
    session: AdnlUdpSession,
    overlay: Option<OverlayId>,
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
                            match tl_proto::deserialize::<OverlayBroadcast>(&data[36..]) {
                                Ok(OverlayBroadcast::Unicast { data }) => data,
                                Ok(broadcast) => broadcast
                                    .payload_if_valid(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs()
                                            .min(i32::MAX as u64)
                                            as i32,
                                    )
                                    .ok_or_else(|| "invalid overlay broadcast".to_owned())?
                                    .to_vec(),
                                Err(_) => return Err("invalid overlay payload".to_owned()),
                            }
                        } else {
                            match tl_proto::deserialize::<OverlayBroadcast>(&data) {
                                Ok(OverlayBroadcast::Unicast { data }) => data,
                                Ok(broadcast) => broadcast
                                    .payload_if_valid(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs()
                                            .min(i32::MAX as u64)
                                            as i32,
                                    )
                                    .ok_or_else(|| "invalid overlay broadcast".to_owned())?
                                    .to_vec(),
                                Err(_) => data,
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
