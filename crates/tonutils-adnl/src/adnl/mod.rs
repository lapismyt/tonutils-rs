//! Thanks to <https://github.com/tonstack/adnl-rs>

pub mod crypto;
pub mod helper_types;
pub mod primitives;
pub mod wrappers;

#[cfg(feature = "udp")]
pub mod udp;

#[cfg(feature = "quic")]
pub mod quic;

#[cfg(test)]
mod tests;

pub use crypto::{KeyPair, PublicKey};
pub use helper_types::{AdnlAddress, AdnlAesParams, AdnlConnectionInfo, AdnlError};
pub use primitives::codec::AdnlCodec;
pub use primitives::handshake::AdnlHandshake;
#[cfg(feature = "quic")]
pub use quic::{QuicServer, QuicSession, sni_for_public_key};
#[cfg(feature = "udp")]
pub use udp::{
    AdnlChannelCipher, AdnlChannelPacket, AdnlUdpPeer, AdnlUdpSession, AdnlUdpSocket,
    channel_id_for_secret, decrypt_direct, encrypt_direct, now_i32, ordered_channel_ciphers,
    reverse_channel_secret,
};
pub use wrappers::builder::AdnlBuilder;
pub use wrappers::peer::AdnlPeer;
