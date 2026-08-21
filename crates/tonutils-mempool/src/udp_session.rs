use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use tonutils_adnl::{AdnlUdpSession, KeyPair, PublicKey as AdnlPublicKey};
use tonutils_overlay::{OverlaySession, PeerId};
use tonutils_tl::Message as AdnlMessage;
use tonutils_tl::tl::network::PacketContents;

/// Adapter exposing an authenticated direct ADNL UDP session to the overlay.
pub struct AdnlUdpOverlaySession {
    peer: PeerId,
    session: AdnlUdpSession,
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
        Ok(Self { peer, session })
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
        Ok(Self { peer, session })
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
                        return Ok(Arc::from(data));
                    }
                }
            }
        })
    }

    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.session
                .send_contents(PacketContents {
                    rand1: vec![0; 7],
                    flags: (),
                    from: None,
                    from_short: None,
                    message: Some(AdnlMessage::Custom {
                        data: payload.to_vec(),
                    }),
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
