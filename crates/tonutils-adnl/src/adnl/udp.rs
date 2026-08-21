//! Datagram framing for ADNL sessions.
//!
//! UDP is deliberately exposed as a datagram primitive.  Handshake and peer
//! discovery remain owned by the caller because a UDP endpoint can be shared
//! by several ADNL peers and the protocol does not provide stream semantics.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio_util::bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::{AdnlAesParams, AdnlCodec, AdnlError};

/// Maximum encoded ADNL datagram accepted by the native UDP helper.
pub const MAX_UDP_PACKET_SIZE: usize = 64 * 1024;

/// An encrypted ADNL datagram peer bound to one remote endpoint.
pub struct AdnlUdpPeer {
    remote: SocketAddr,
    codec: AdnlCodec,
    seen: VecDeque<[u8; 32]>,
}

impl AdnlUdpPeer {
    /// Creates a UDP peer using client-side session keys.
    pub fn client(remote: SocketAddr, params: &AdnlAesParams) -> Self {
        Self {
            remote,
            codec: AdnlCodec::client(params),
            seen: VecDeque::new(),
        }
    }

    /// Creates a UDP peer using server-side session keys.
    pub fn server(remote: SocketAddr, params: &AdnlAesParams) -> Self {
        Self {
            remote,
            codec: AdnlCodec::server(params),
            seen: VecDeque::new(),
        }
    }

    pub fn remote(&self) -> SocketAddr {
        self.remote
    }

    /// Encodes one payload as exactly one UDP datagram.
    pub fn encode(&mut self, payload: Bytes) -> Result<Bytes, AdnlError> {
        let mut output = BytesMut::new();
        self.codec.encode(payload, &mut output)?;
        if output.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        Ok(output.freeze())
    }

    /// Decodes one complete UDP datagram and rejects trailing frames.
    pub fn decode(&mut self, datagram: &[u8]) -> Result<Bytes, AdnlError> {
        if datagram.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        let packet_hash: [u8; 32] = Sha256::digest(datagram).into();
        if self.seen.contains(&packet_hash) {
            return Err(AdnlError::ReplayDetected);
        }
        let mut input = BytesMut::from(datagram);
        let payload = self
            .codec
            .decode(&mut input)?
            .ok_or(AdnlError::EndOfStream)?;
        if !input.is_empty() {
            return Err(AdnlError::TooLongPacket);
        }
        self.seen.push_back(packet_hash);
        if self.seen.len() > 4096 {
            self.seen.pop_front();
        }
        Ok(payload)
    }
}

/// Tokio UDP endpoint for one authenticated ADNL datagram session.
pub struct AdnlUdpSocket {
    socket: tokio::net::UdpSocket,
    peer: AdnlUdpPeer,
}

impl AdnlUdpSocket {
    pub async fn bind(
        local: SocketAddr,
        remote: SocketAddr,
        params: &AdnlAesParams,
    ) -> Result<Self, AdnlError> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        socket.connect(remote).await?;
        Ok(Self {
            socket,
            peer: AdnlUdpPeer::client(remote, params),
        })
    }

    pub fn remote(&self) -> SocketAddr {
        self.peer.remote()
    }

    pub async fn send(&mut self, payload: Bytes) -> Result<usize, AdnlError> {
        let packet = self.peer.encode(payload)?;
        Ok(self.socket.send(&packet).await?)
    }

    pub async fn recv(&mut self) -> Result<Bytes, AdnlError> {
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE];
        let size = self.socket.recv(&mut packet).await?;
        self.peer.decode(&packet[..size])
    }

    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<Bytes, AdnlError> {
        tokio::time::timeout(timeout, self.recv())
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP receive",
                timeout,
            })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_reject_trailing_data() {
        let params = AdnlAesParams::default();
        let address = "127.0.0.1:30303".parse().unwrap();
        let mut client = AdnlUdpPeer::client(address, &params);
        let mut server = AdnlUdpPeer::server(address, &params);
        let packet = client.encode(Bytes::from_static(&[1, 2, 3])).unwrap();
        assert_eq!(server.decode(&packet).unwrap().as_ref(), [1, 2, 3]);
        assert!(matches!(
            server.decode(&packet),
            Err(AdnlError::ReplayDetected)
        ));

        let mut malformed = packet.to_vec();
        malformed.push(0);
        assert!(server.decode(&malformed).is_err());
    }
}
