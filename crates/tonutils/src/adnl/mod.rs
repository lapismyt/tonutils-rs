//! Thanks to <https://github.com/tonstack/adnl-rs>

pub mod crypto;
pub mod helper_types;
pub mod primitives;
pub mod quic;
pub mod tl;
pub mod udp;
pub mod wrappers;

#[cfg(test)]
mod tests;

pub use crypto::{AdnlKeyPair, AdnlPublicKey};
pub use helper_types::{AdnlAddress, AdnlAesParams, AdnlConnectionInfo, AdnlError};
pub use primitives::codec::AdnlCodec;
pub use primitives::handshake::AdnlHandshake;
pub use quic::{QuicServer, QuicSession, tls::sni_for_public_key};
pub use udp::{
    AdnlChannelCipher, AdnlChannelPacket, AdnlUdpPeer, AdnlUdpSession, AdnlUdpSocket,
    AdnlUdpTransport, channel_id_for_secret, decrypt_direct, encrypt_direct, now_i32,
    ordered_channel_ciphers, reverse_channel_secret,
};
pub use wrappers::builder::AdnlBuilder;
pub use wrappers::peer::AdnlPeer;
