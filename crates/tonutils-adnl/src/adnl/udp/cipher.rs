//! Direct-packet and channel-packet cryptography for ADNL over UDP.
//!
//! Everything here is a pure encoding step: no sockets, no keys of its own
//! beyond the ones handed to it, and no I/O.  Split out of the transport module
//! so the framing rules can be read without the session state machine.

use std::collections::VecDeque;

use aes::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio_util::bytes::Bytes;
use tonutils_tl::tl::network::PacketContents;

use crate::AdnlError;
use crate::crypto::{KeyPair, PublicKey};

use super::MAX_UDP_PACKET_SIZE;

/// AES-CTR channel cipher used after an ADNL channel is established.
///
/// Channel packets carry a 32-byte SHA-256 digest followed by ciphertext.
/// The per-packet key and IV are derived from that digest and the channel
/// secret, matching the upstream TON `EncryptorAES`/`DecryptorAES` layout.
#[derive(Clone)]
pub struct AdnlChannelCipher {
    secret: [u8; 32],
}

impl AdnlChannelCipher {
    #[must_use]
    pub fn new(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    #[must_use]
    pub fn secret(&self) -> [u8; 32] {
        self.secret
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Bytes {
        let digest: [u8; 32] = Sha256::digest(plaintext).into();
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(&self.secret[..16]);
        key[16..].copy_from_slice(&digest[16..]);
        let mut iv = [0u8; 16];
        iv[..4].copy_from_slice(&digest[..4]);
        iv[4..].copy_from_slice(&self.secret[20..]);
        let mut ciphertext = plaintext.to_vec();
        ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into())
            .apply_keystream(&mut ciphertext);
        let mut output = Vec::with_capacity(32 + ciphertext.len());
        output.extend_from_slice(&digest);
        output.extend_from_slice(&ciphertext);
        Bytes::from(output)
    }

    pub fn decrypt(&self, packet: &[u8]) -> Result<Bytes, AdnlError> {
        if packet.len() < 32 {
            return Err(AdnlError::TooShortPacket);
        }
        let digest: [u8; 32] = packet[..32]
            .try_into()
            .map_err(|_| AdnlError::TooShortPacket)?;
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(&self.secret[..16]);
        key[16..].copy_from_slice(&digest[16..]);
        let mut iv = [0u8; 16];
        iv[..4].copy_from_slice(&digest[..4]);
        iv[4..].copy_from_slice(&self.secret[20..]);
        let mut plaintext = packet[32..].to_vec();
        ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into())
            .apply_keystream(&mut plaintext);
        if !bool::from(Sha256::digest(&plaintext).as_slice().ct_eq(&digest)) {
            return Err(AdnlError::IntegrityError);
        }
        Ok(Bytes::from(plaintext))
    }
}

#[must_use]
pub fn reverse_channel_secret(mut secret: [u8; 32]) -> [u8; 32] {
    secret.reverse();
    secret
}

#[must_use]
pub fn ordered_channel_ciphers(
    local_id: [u8; 32],
    peer_id: [u8; 32],
    shared_secret: [u8; 32],
) -> (AdnlChannelCipher, AdnlChannelCipher) {
    let reversed = reverse_channel_secret(shared_secret);
    if local_id <= peer_id {
        (
            AdnlChannelCipher::new(reversed),
            AdnlChannelCipher::new(shared_secret),
        )
    } else {
        (
            AdnlChannelCipher::new(shared_secret),
            AdnlChannelCipher::new(reversed),
        )
    }
}

#[must_use]
pub fn channel_id_for_secret(secret: [u8; 32]) -> [u8; 32] {
    let mut public_key = Vec::with_capacity(36);
    public_key.extend_from_slice(&0x2dbcadd4u32.to_le_bytes());
    public_key.extend_from_slice(&secret);
    Sha256::digest(public_key).into()
}

/// Encodes and validates packets carried by an established ADNL channel.
pub struct AdnlChannelPacket {
    /// Channel id we advertise when sending, and expect when receiving.
    pub(super) outbound_id: [u8; 32],
    /// Channel id the peer advertises when sending.
    pub(super) inbound_id: [u8; 32],
    outbound: AdnlChannelCipher,
    inbound: AdnlChannelCipher,
    /// Next outbound sequence number, primed from the peer-pair state so a
    /// re-created packet does not restart a sequence the peer already saw.
    pub(super) next_seqno: u64,
    /// Highest inbound sequence number seen on this channel.
    pub(super) highest_seqno: u64,
    received: VecDeque<u64>,
}

fn aes_encrypt(secret: [u8; 32], plaintext: &[u8]) -> Bytes {
    let digest: [u8; 32] = Sha256::digest(plaintext).into();
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&secret[..16]);
    key[16..].copy_from_slice(&digest[16..]);
    let iv: [u8; 16] = [&digest[..4], &secret[20..]]
        .concat()
        .try_into()
        .expect("digest prefix and secret suffix must form a 16-byte IV");
    let mut ciphertext = plaintext.to_vec();
    ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into()).apply_keystream(&mut ciphertext);
    let mut output = Vec::with_capacity(32 + ciphertext.len());
    output.extend_from_slice(&digest);
    output.extend_from_slice(&ciphertext);
    Bytes::from(output)
}

fn aes_decrypt(secret: [u8; 32], packet: &[u8]) -> Result<Bytes, AdnlError> {
    if packet.len() < 32 {
        return Err(AdnlError::TooShortPacket);
    }
    let digest: [u8; 32] = packet[..32]
        .try_into()
        .map_err(|_| AdnlError::TooShortPacket)?;
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&secret[..16]);
    key[16..].copy_from_slice(&digest[16..]);
    let iv: [u8; 16] = [&digest[..4], &secret[20..]]
        .concat()
        .try_into()
        .map_err(|_| AdnlError::IntegrityError)?;
    let mut plaintext = packet[32..].to_vec();
    ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into()).apply_keystream(&mut plaintext);
    if !bool::from(Sha256::digest(&plaintext).as_slice().ct_eq(&digest)) {
        return Err(AdnlError::IntegrityError);
    }
    Ok(Bytes::from(plaintext))
}

/// Direct ADNL packet encryption used before an optional channel is ready.
pub fn encrypt_direct(remote: &PublicKey, plaintext: &[u8]) -> Bytes {
    let ephemeral = KeyPair::generate(&mut rand::rngs::OsRng);
    let encrypted = aes_encrypt(ephemeral.compute_shared_secret(remote), plaintext);
    let mut output = Vec::with_capacity(32 + encrypted.len());
    output.extend_from_slice(ephemeral.public_key.as_bytes());
    output.extend_from_slice(&encrypted);
    Bytes::from(output)
}

/// Decrypts a direct ADNL packet and returns the sender's ephemeral key.
pub fn decrypt_direct(local: &KeyPair, packet: &[u8]) -> Result<(PublicKey, Bytes), AdnlError> {
    if packet.len() < 64 {
        return Err(AdnlError::TooShortPacket);
    }
    let public = PublicKey::from_bytes(
        packet[..32]
            .try_into()
            .map_err(|_| AdnlError::InvalidPublicKey)?,
    )
    .ok_or(AdnlError::InvalidPublicKey)?;
    Ok((
        public,
        aes_decrypt(local.compute_shared_secret(&public), &packet[32..])?,
    ))
}

impl AdnlChannelPacket {
    #[must_use]
    pub fn new(
        channel_id: [u8; 32],
        outbound: AdnlChannelCipher,
        inbound: AdnlChannelCipher,
    ) -> Self {
        Self::new_directional(channel_id, channel_id, outbound, inbound)
    }

    #[must_use]
    pub fn new_directional(
        outbound_id: [u8; 32],
        inbound_id: [u8; 32],
        outbound: AdnlChannelCipher,
        inbound: AdnlChannelCipher,
    ) -> Self {
        Self {
            outbound_id,
            inbound_id,
            outbound,
            inbound,
            next_seqno: 0,
            highest_seqno: 0,
            received: VecDeque::new(),
        }
    }

    #[must_use]
    pub fn channel_id(&self) -> [u8; 32] {
        self.outbound_id
    }

    /// Encrypts `contents` for an established channel.
    ///
    /// `seqno` and `confirm_seqno` are assigned from the channel's own
    /// counters when the caller left them unset.  [`AdnlUdpSession`] always
    /// sets them first, because the peer pair - not the channel - owns that
    /// counter; standalone users keep the channel-local behaviour.
    pub fn encode(&mut self, mut contents: PacketContents) -> Result<Bytes, AdnlError> {
        if contents.message.is_none() && contents.messages.is_none() {
            return Err(AdnlError::InvalidPacket);
        }
        if contents.seqno.is_none() {
            self.next_seqno = self.next_seqno.saturating_add(1);
            contents.seqno = Some(self.next_seqno);
        }
        if contents.confirm_seqno.is_none() {
            contents.confirm_seqno = Some(self.highest_seqno);
        }
        let payload = tl_proto::serialize(contents);
        let encrypted = self.outbound.encrypt(&payload);
        if encrypted.len() + self.outbound_id.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        let mut packet = Vec::with_capacity(self.outbound_id.len() + encrypted.len());
        packet.extend_from_slice(&self.outbound_id);
        packet.extend_from_slice(&encrypted);
        Ok(Bytes::from(packet))
    }

    pub fn decode(&mut self, datagram: &[u8]) -> Result<PacketContents, AdnlError> {
        if datagram.len() < self.inbound_id.len() + 32
            || datagram.len() > MAX_UDP_PACKET_SIZE
            || datagram[..self.inbound_id.len()] != self.inbound_id
        {
            return Err(AdnlError::InvalidPacket);
        }
        let payload = self.inbound.decrypt(&datagram[self.inbound_id.len()..])?;
        let contents: PacketContents = tl_proto::deserialize(&payload).map_err(|error| {
            let prefix = hex::encode(payload.iter().take(96).copied().collect::<Vec<_>>());
            AdnlError::MalformedPacket(format!("{error} (channel payload={prefix})"))
        })?;
        if let Some(flags) = super::raw_packet_flags(&payload) {
            log::debug!(
                "flags probe: kind=channel chan={} raw=0x{flags:08x} recv_v={:?} recv_prio={:?}",
                hex::encode(&self.inbound_id[..8]),
                contents.recv_addr_list_version,
                contents.recv_priority_addr_list_version,
            );
        }
        if let Some(confirm_seqno) = contents.confirm_seqno
            && confirm_seqno > self.next_seqno
        {
            return Err(AdnlError::ReplayDetected);
        }
        if let Some(seqno) = contents.seqno {
            if seqno == 0
                || self.received.contains(&seqno)
                || (self.highest_seqno > 4096 && seqno + 4096 < self.highest_seqno)
            {
                return Err(AdnlError::ReplayDetected);
            }
            self.highest_seqno = self.highest_seqno.max(seqno);
            self.received.push_back(seqno);
            while self.received.len() > 4096 {
                self.received.pop_front();
            }
        }
        Ok(contents)
    }
}
