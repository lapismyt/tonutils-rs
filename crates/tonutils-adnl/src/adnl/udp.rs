//! Datagram framing for ADNL sessions.
//!
//! UDP is deliberately exposed as a datagram primitive.  Handshake and peer
//! discovery remain owned by the caller because a UDP endpoint can be shared
//! by several ADNL peers and the protocol does not provide stream semantics.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use aes::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha256};
use tokio_util::bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tonutils_tl::tl::network::{
    AddressList, DhtMessage, DhtNodesBoxed, DhtValueResult, OverlayNodes, OverlayNodesBoxed,
    OverlayQuery, PacketContents, PublicKey as TlPublicKey,
};
use tonutils_tl::{Int256, Message as AdnlMessage};

use crate::crypto::{KeyPair, PublicKey};
use crate::{AdnlAddress, AdnlAesParams, AdnlCodec, AdnlError};

/// Maximum encoded ADNL datagram accepted by the native UDP helper.
pub const MAX_UDP_PACKET_SIZE: usize = 64 * 1024;

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
        if Sha256::digest(&plaintext).as_slice() != digest {
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
    outbound_id: [u8; 32],
    inbound_id: [u8; 32],
    outbound: AdnlChannelCipher,
    inbound: AdnlChannelCipher,
    next_seqno: u64,
    highest_seqno: u64,
    received: VecDeque<u64>,
}

fn aes_encrypt(secret: [u8; 32], plaintext: &[u8]) -> Bytes {
    let digest: [u8; 32] = Sha256::digest(plaintext).into();
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&secret[..16]);
    key[16..].copy_from_slice(&digest[16..]);
    let mut iv = [0u8; 16];
    iv[..4].copy_from_slice(&digest[..4]);
    iv[4..].copy_from_slice(&secret[20..]);
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
    let mut iv = [0u8; 16];
    iv[..4].copy_from_slice(&digest[..4]);
    iv[4..].copy_from_slice(&secret[20..]);
    let mut plaintext = packet[32..].to_vec();
    ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into()).apply_keystream(&mut plaintext);
    if Sha256::digest(&plaintext).as_slice() != digest {
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

/// Authenticated UDP ADNL endpoint for direct packets and established channels.
pub struct AdnlUdpSession {
    socket: tokio::net::UdpSocket,
    local: KeyPair,
    remote: PublicKey,
    local_id: [u8; 32],
    remote_id: [u8; 32],
    channel: Option<AdnlChannelPacket>,
    pending_channel: Option<(KeyPair, i32)>,
    next_seqno: u64,
    received: VecDeque<u64>,
    highest_seqno: u64,
}

impl AdnlUdpSession {
    pub async fn connect(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local: KeyPair,
        remote: PublicKey,
    ) -> Result<Self, AdnlError> {
        let socket = tokio::net::UdpSocket::bind(local_addr).await?;
        socket.connect(remote_addr).await?;
        Ok(Self {
            socket,
            local_id: AdnlAddress::from(&local.public_key).to_bytes(),
            remote_id: AdnlAddress::from(&remote).to_bytes(),
            local,
            remote,
            channel: None,
            pending_channel: None,
            next_seqno: 0,
            received: VecDeque::new(),
            highest_seqno: 0,
        })
    }

    pub async fn connect_with_channel(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local: KeyPair,
        remote: PublicKey,
        timeout: Duration,
    ) -> Result<Self, AdnlError> {
        let mut session = Self::connect(local_addr, remote_addr, local, remote).await?;
        session.establish_channel(timeout).await?;
        Ok(session)
    }

    pub async fn send_contents(&mut self, contents: PacketContents) -> Result<usize, AdnlError> {
        if let Some(channel) = self.channel.as_mut() {
            let packet = channel.encode(contents)?;
            return Ok(self.socket.send(&packet).await?);
        }
        self.send_direct_contents(contents).await
    }

    pub async fn send_answer(
        &mut self,
        query_id: Int256,
        answer: Vec<u8>,
    ) -> Result<usize, AdnlError> {
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Answer { query_id, answer }),
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
    }

    async fn send_direct_contents(
        &mut self,
        mut contents: PacketContents,
    ) -> Result<usize, AdnlError> {
        contents.from = Some(TlPublicKey::Ed25519 {
            key: tonutils_tl::Int256(self.local.public_key.to_bytes()),
        });
        self.next_seqno = self.next_seqno.saturating_add(1);
        contents.seqno = Some(self.next_seqno);
        contents.confirm_seqno = Some(self.highest_seqno);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i32::MAX as u64) as i32;
        contents.reinit_date.get_or_insert(now);
        contents.dst_reinit_date.get_or_insert(0);
        let mut unsigned = contents.clone();
        unsigned.signature = None;
        let signature = self.local.sign_raw(&tl_proto::serialize(unsigned));
        contents.signature = Some(signature.to_vec());
        let encrypted = encrypt_direct(&self.remote, &tl_proto::serialize(contents));
        let mut packet = Vec::with_capacity(32 + encrypted.len());
        packet.extend_from_slice(&self.remote_id);
        packet.extend_from_slice(&encrypted);
        if packet.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        Ok(self.socket.send(&packet).await?)
    }

    #[allow(clippy::unnecessary_join)]
    pub async fn recv_contents(&mut self) -> Result<PacketContents, AdnlError> {
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
        loop {
            let size = self.socket.recv(&mut packet).await?;
            if size > MAX_UDP_PACKET_SIZE {
                return Err(AdnlError::TooLongPacket);
            }
            if let Some(channel) = self.channel.as_mut()
                && size >= channel.inbound_id.len()
                && packet[..channel.inbound_id.len()] == channel.inbound_id
            {
                match channel.decode(&packet[..size]) {
                    Ok(contents) => return Ok(contents),
                    Err(error) => log::debug!("dropping invalid ADNL channel packet: {error}"),
                }
                continue;
            }
            if size < self.local_id.len() || packet[..self.local_id.len()] != self.local_id {
                continue;
            }
            let Ok((_, payload)) = decrypt_direct(&self.local, &packet[32..size]) else {
                continue;
            };
            let Ok(contents) = tl_proto::deserialize::<PacketContents>(&payload) else {
                continue;
            };
            let Some(TlPublicKey::Ed25519 { key }) = &contents.from else {
                continue;
            };
            let Some(sender) = PublicKey::from_bytes(key.0) else {
                continue;
            };
            if sender != self.remote {
                continue;
            }
            if let Some(from_short) = &contents.from_short
                && from_short.id.0 != self.remote_id
            {
                continue;
            }
            let Some(signature) = &contents.signature else {
                continue;
            };
            let Ok(signature) = signature.as_slice().try_into() else {
                continue;
            };
            let mut unsigned = contents.clone();
            unsigned.signature = None;
            if !self
                .remote
                .verify_raw(&tl_proto::serialize(unsigned), &signature)
            {
                continue;
            }
            if let Some(seqno) = contents.seqno {
                if seqno == 0
                    || self.received.contains(&seqno)
                    || (self.highest_seqno > 4096 && seqno + 4096 < self.highest_seqno)
                {
                    continue;
                }
                self.highest_seqno = self.highest_seqno.max(seqno);
                self.received.push_back(seqno);
                while self.received.len() > 4096 {
                    self.received.pop_front();
                }
            }
            if self.process_channel_control(&contents).await.is_err() {
                continue;
            }
            return Ok(contents);
        }
    }

    async fn process_channel_control(
        &mut self,
        contents: &PacketContents,
    ) -> Result<(), AdnlError> {
        let messages = contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten());
        for message in messages {
            match message {
                AdnlMessage::CreateChannel { key, date } => {
                    let Some(peer_channel) = PublicKey::from_bytes(key.0) else {
                        return Err(AdnlError::InvalidPublicKey);
                    };
                    let local_channel = KeyPair::generate(&mut rand::rngs::OsRng);
                    self.install_channel(&local_channel, peer_channel.to_bytes(), *date)?;
                    self.send_direct_contents(PacketContents {
                        rand1: vec![0; 7],
                        flags: (),
                        from: None,
                        from_short: None,
                        message: Some(AdnlMessage::ConfirmChannel {
                            key: Int256(local_channel.public_key.to_bytes()),
                            peer_key: key.clone(),
                            date: *date,
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
                    .await?;
                }
                AdnlMessage::ConfirmChannel {
                    key,
                    peer_key,
                    date,
                } => {
                    let Some((local_channel, local_date)) = self.pending_channel.take() else {
                        continue;
                    };
                    if peer_key.0 != local_channel.public_key.to_bytes() || *date < local_date {
                        return Err(AdnlError::InvalidPacket);
                    }
                    self.install_channel(&local_channel, key.0, *date)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn install_channel(
        &mut self,
        local_channel: &KeyPair,
        remote_channel: [u8; 32],
        _date: i32,
    ) -> Result<(), AdnlError> {
        let remote_channel =
            PublicKey::from_bytes(remote_channel).ok_or(AdnlError::InvalidPublicKey)?;
        let shared = local_channel.compute_shared_secret(&remote_channel);
        let (outbound, inbound) = ordered_channel_ciphers(self.local_id, self.remote_id, shared);
        let outbound_id = channel_id_for_secret(outbound.secret());
        let inbound_id = channel_id_for_secret(inbound.secret());
        self.channel = Some(AdnlChannelPacket::new_directional(
            outbound_id,
            inbound_id,
            outbound,
            inbound,
        ));
        Ok(())
    }

    pub async fn establish_channel(&mut self, timeout: Duration) -> Result<(), AdnlError> {
        if self.channel.is_some() {
            return Ok(());
        }
        let local_channel = KeyPair::generate(&mut rand::rngs::OsRng);
        let date = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i32::MAX as u64) as i32;
        self.pending_channel = Some((local_channel, date));
        self.send_direct_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: None,
            messages: Some(vec![
                AdnlMessage::CreateChannel {
                    key: Int256(local_channel.public_key.to_bytes()),
                    date,
                },
                AdnlMessage::Query {
                    query_id: Int256::random(),
                    query: tl_proto::serialize(DhtMessage::GetSignedAddressList),
                },
            ]),
            address: Some(AddressList {
                addrs: Vec::new(),
                version: date,
                reinit_date: date,
                priority: 0,
                expire_at: 0,
            }),
            priority_address: None,
            seqno: None,
            confirm_seqno: None,
            recv_addr_list_version: Some(date),
            recv_priority_addr_list_version: None,
            reinit_date: None,
            dst_reinit_date: Some(0),
            signature: None,
            rand2: vec![0; 7],
        })
        .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        while self.channel.is_none() {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "ADNL UDP channel handshake",
                    timeout,
                });
            }
            let _ = self.recv_timeout(remaining).await?;
        }
        Ok(())
    }

    pub async fn send_timeout(
        &mut self,
        contents: PacketContents,
        timeout: Duration,
    ) -> Result<usize, AdnlError> {
        tokio::time::timeout(timeout, self.send_contents(contents))
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP packet send",
                timeout,
            })?
    }

    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<PacketContents, AdnlError> {
        tokio::time::timeout(timeout, self.recv_contents())
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP packet receive",
                timeout,
            })?
    }

    #[allow(clippy::unnecessary_join)]
    pub async fn dht_find_node(
        &mut self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindNode { key, k: count });
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
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
        .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "DHT findNode",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    let nodes: DhtNodesBoxed = tl_proto::deserialize(&answer).map_err(|error| {
                        let prefix = answer
                            .iter()
                            .take(32)
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<Vec<_>>()
                            .join("");
                        eprintln!(
                            "dht_find_node: deserialize error: {error} len={} answer={prefix}",
                            answer.len()
                        );
                        AdnlError::MalformedPacket(format!(
                            "{error} len={} answer={prefix}",
                            answer.len()
                        ))
                    })?;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .min(i32::MAX as u64) as i32;
                    let total = nodes.nodes.len();
                    let mut selected = Vec::with_capacity(total);
                    for node in &nodes.nodes {
                        if node.is_valid(now) {
                            selected.push(node.clone());
                        } else {
                            eprintln!(
                                "dht_find_node: filtered node version={} expire_at={} addrs={} sig_len={}",
                                node.version,
                                node.addr_list.expire_at,
                                node.addr_list.addrs.len(),
                                node.signature.len(),
                            );
                        }
                    }
                    eprintln!(
                        "dht_find_node: received {total} raw nodes, {}/{} passed is_valid",
                        selected.len(),
                        total,
                    );
                    return Ok(DhtNodesBoxed { nodes: selected });
                }
            }
        }
    }

    pub async fn dht_find_value(
        &mut self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtValueResult, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindValue { key, k: count });
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
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
        .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "DHT findValue",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    let prefix: String =
                        answer.iter().take(16).map(|b| format!("{b:02x}")).collect();
                    eprintln!(
                        "dht_find_value: got answer len={} prefix={prefix}",
                        answer.len()
                    );
                    return tl_proto::deserialize(&answer).map_err(|error| {
                        eprintln!("dht_find_value: deserialize error: {error}");
                        AdnlError::MalformedPacket(error.to_string())
                    });
                }
            }
        }
    }

    pub async fn overlay_get_random_peers(
        &mut self,
        overlay: Int256,
        timeout: Duration,
    ) -> Result<OverlayNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        self.send_overlay_get_random_peers_with_id(overlay, query_id.clone())
            .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "overlay getRandomPeers",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    return tl_proto::deserialize(&answer)
                        .map_err(|error| AdnlError::MalformedPacket(error.to_string()));
                }
            }
        }
    }

    pub async fn send_overlay_get_random_peers(
        &mut self,
        overlay: Int256,
    ) -> Result<usize, AdnlError> {
        self.send_overlay_get_random_peers_with_id(overlay, Int256::random())
            .await
    }

    async fn send_overlay_get_random_peers_with_id(
        &mut self,
        overlay: Int256,
        query_id: Int256,
    ) -> Result<usize, AdnlError> {
        let mut query = tl_proto::serialize(OverlayQuery::Query {
            overlay: overlay.clone(),
        });
        query.extend(tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: OverlayNodes {
                nodes: vec![self.local_overlay_node(overlay)],
            },
        }));
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
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
    }

    fn local_overlay_node(&self, overlay: Int256) -> tonutils_tl::tl::network::OverlayNode {
        let version = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i32::MAX as u64) as i32;
        let to_sign = tonutils_tl::tl::network::OverlayNodeToSign {
            id: tonutils_tl::tl::network::AdnlIdShort {
                id: Int256(self.local_id),
            },
            overlay: overlay.clone(),
            version,
        };
        tonutils_tl::tl::network::OverlayNode {
            id: tonutils_tl::tl::network::PublicKey::Ed25519 {
                key: Int256(self.local.public_key.to_bytes()),
            },
            overlay,
            version,
            signature: self.local.sign_raw(&tl_proto::serialize(to_sign)).to_vec(),
        }
    }
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

    pub fn encode(&mut self, mut contents: PacketContents) -> Result<Bytes, AdnlError> {
        if contents.message.is_none() && contents.messages.is_none() {
            return Err(AdnlError::InvalidPacket);
        }
        self.next_seqno = self.next_seqno.saturating_add(1);
        contents.seqno = Some(self.next_seqno);
        contents.confirm_seqno = Some(self.highest_seqno);
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

    pub async fn send_timeout(
        &mut self,
        payload: Bytes,
        timeout: Duration,
    ) -> Result<usize, AdnlError> {
        tokio::time::timeout(timeout, self.send(payload))
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP send",
                timeout,
            })?
    }

    pub async fn recv(&mut self) -> Result<Bytes, AdnlError> {
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
        let size = self.socket.recv(&mut packet).await?;
        if size > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
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
#[cfg(test)]
#[path = "udp_tests.rs"]
mod tests;
