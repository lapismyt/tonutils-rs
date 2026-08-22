//! Canonical ADNL, DHT, and overlay wire values used by peer discovery.

use derivative::Derivative;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use tl_proto::{TlRead, TlWrite};

use super::adnl::Message as AdnlMessage;
use super::common::Int256;

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct Int128(pub i32, pub i32, pub i32, pub i32);

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct AdnlIdShort {
    pub id: Int256,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum PublicKey {
    /// pub.ed25519 key:int256 = PublicKey;
    #[tl(id = 0x4813b4c6)]
    Ed25519 { key: Int256 },
    /// pub.aes key:int256 = PublicKey;
    #[tl(id = 0x2dbcadd4)]
    Aes { key: Int256 },
    /// pub.unenc data:bytes = PublicKey;
    #[tl(id = 0xb61f450a)]
    Unencoded { data: Vec<u8> },
    /// pub.overlay name:bytes = PublicKey;
    #[tl(id = 0x34ba45cb)]
    Overlay { name: Vec<u8> },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum Address {
    /// adnl.address.udp ip:int port:int = adnl.Address;
    #[tl(id = 0x670da6e7)]
    Udp { ip: i32, port: i32 },
    /// adnl.address.udp6 ip:int128 port:int = adnl.Address;
    #[tl(id = 0xe31d63fa)]
    Udp6 { ip: Int128, port: i32 },
    /// adnl.address.tunnel to:int256 pubkey:PublicKey = adnl.Address;
    #[tl(id = 0x092b02eb)]
    Tunnel { to: Int256, pubkey: PublicKey },
    /// adnl.address.reverse = adnl.Address;
    #[tl(id = 0x27795286)]
    Reverse,
    /// adnl.address.quic ip:int port:int = adnl.Address;
    #[tl(id = 0x78017253)]
    Quic { ip: i32, port: i32 },
}

impl Address {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Udp { ip, port } | Self::Quic { ip, port } => *port > 0 && *ip != 0,
            Self::Udp6 { ip, port } => *port > 0 && (ip.0, ip.1, ip.2, ip.3) != (0, 0, 0, 0),
            Self::Tunnel { .. } | Self::Reverse => false,
        }
    }
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(
    boxed,
    id = 0xd142cd89,
    scheme_inline = r##"adnl.packetContents rand1:bytes flags:# from:flags.0?PublicKey from_short:flags.1?adnl.id.short message:flags.2?adnl.Message messages:flags.3?(vector adnl.Message) address:flags.4?adnl.addressList priority_address:flags.5?adnl.addressList seqno:flags.6?long confirm_seqno:flags.7?long recv_addr_list_version:flags.8?int recv_priority_addr_list_version:flags.9?int reinit_date:flags.10?int dst_reinit_date:flags.10?int signature:flags.11?bytes rand2:bytes = adnl.PacketContents;"##
)]
pub struct PacketContents {
    pub rand1: Vec<u8>,
    #[tl(flags)]
    pub flags: (),
    #[tl(flags_bit = "flags.0")]
    pub from: Option<PublicKey>,
    #[tl(flags_bit = "flags.1")]
    pub from_short: Option<AdnlIdShort>,
    #[tl(flags_bit = "flags.2")]
    pub message: Option<AdnlMessage>,
    #[tl(flags_bit = "flags.3")]
    pub messages: Option<Vec<AdnlMessage>>,
    #[tl(flags_bit = "flags.4")]
    pub address: Option<AddressList>,
    #[tl(flags_bit = "flags.5")]
    pub priority_address: Option<AddressList>,
    #[tl(flags_bit = "flags.6")]
    pub seqno: Option<u64>,
    #[tl(flags_bit = "flags.7")]
    pub confirm_seqno: Option<u64>,
    #[tl(flags_bit = "flags.8")]
    pub recv_addr_list_version: Option<i32>,
    #[tl(flags_bit = "flags.9")]
    pub recv_priority_addr_list_version: Option<i32>,
    #[tl(flags_bit = "flags.10")]
    pub reinit_date: Option<i32>,
    #[tl(flags_bit = "flags.10")]
    pub dst_reinit_date: Option<i32>,
    #[tl(flags_bit = "flags.11")]
    pub signature: Option<Vec<u8>>,
    pub rand2: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct AddressList {
    /// adnl.addressList addrs:(vector adnl.Address) version:int reinit_date:int
    /// priority:int expire_at:int = adnl.AddressList;
    pub addrs: Vec<Address>,
    pub version: i32,
    pub reinit_date: i32,
    pub priority: i32,
    pub expire_at: i32,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x6b561285)]
pub struct AdnlNode {
    /// adnl.node id:PublicKey addr_list:adnl.addressList = adnl.Node;
    pub id: PublicKey,
    pub addr_list: AddressList,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtNode {
    /// dht.node id:PublicKey addr_list:adnl.addressList version:int signature:bytes = dht.Node;
    pub id: PublicKey,
    pub addr_list: AddressList,
    pub version: i32,
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x84533248)]
pub struct DhtNodeBoxed {
    pub id: PublicKey,
    pub addr_list: AddressList,
    pub version: i32,
    pub signature: Vec<u8>,
}

impl DhtNode {
    #[must_use]
    pub fn is_valid(&self, now: i32) -> bool {
        self.version > 0
            && !self.addr_list.addrs.is_empty()
            && (self.addr_list.expire_at == 0 || self.addr_list.expire_at > now)
            && self.addr_list.addrs.iter().all(Address::is_valid)
            && self.verify_signature()
    }

    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let PublicKey::Ed25519 { key } = &self.id else {
            return false;
        };
        let Ok(public_key) = VerifyingKey::from_bytes(&key.0) else {
            return false;
        };
        let signature_bytes = match self.signature.as_slice() {
            signature if signature.len() == 64 => signature,
            signature if signature.len() == 68 => &signature[4..],
            _ => return false,
        };
        let Ok(signature) = Signature::from_slice(signature_bytes) else {
            return false;
        };
        let unsigned = DhtNodeBoxed {
            id: self.id.clone(),
            addr_list: self.addr_list.clone(),
            version: self.version,
            signature: Vec::new(),
        };
        public_key
            .verify(&tl_proto::serialize(unsigned), &signature)
            .is_ok()
    }
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtNodes {
    /// dht.nodes nodes:(vector dht.node) = dht.Nodes;
    pub nodes: Vec<DhtNode>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x7974a0be)]
pub struct DhtNodesBoxed {
    pub nodes: Vec<DhtNode>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtKey {
    /// dht.key id:int256 name:bytes idx:int = dht.Key;
    pub id: Int256,
    pub name: Vec<u8>,
    pub idx: i32,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum DhtUpdateRule {
    /// dht.updateRule.signature = dht.UpdateRule;
    #[tl(id = 0xcc9f31f7)]
    Signature,
    /// dht.updateRule.anybody = dht.UpdateRule;
    #[tl(id = 0x61578e14)]
    Anybody,
    /// dht.updateRule.overlayNodes = dht.UpdateRule;
    #[tl(id = 0x26779383)]
    OverlayNodes,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum DhtMessage {
    /// dht.ping random_id:long = dht.Pong;
    #[tl(id = 0xcbeb3f18)]
    Ping { random_id: u64 },
    /// dht.store value:dht.value = dht.Stored;
    #[tl(id = 0x34934212)]
    Store { value: DhtValue },
    /// dht.findNode key:int256 k:int = dht.Nodes;
    #[tl(id = 0x6ce2ce6b)]
    FindNode { key: Int256, k: i32 },
    /// dht.findValue key:int256 k:int = dht.ValueResult;
    #[tl(id = 0xae4b6011)]
    FindValue { key: Int256, k: i32 },
}

pub type DhtLookup = DhtMessage;

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtKeyDescription {
    /// dht.keyDescription key:dht.key id:PublicKey update_rule:dht.UpdateRule
    /// signature:bytes = dht.KeyDescription;
    pub key: DhtKey,
    pub id: PublicKey,
    pub update_rule: DhtUpdateRule,
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtValue {
    /// dht.value key:dht.keyDescription value:bytes ttl:int signature:bytes = dht.Value;
    pub key: DhtKeyDescription,
    pub value: Vec<u8>,
    pub ttl: i32,
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct OverlayNode {
    /// overlay.node id:PublicKey overlay:int256 version:int signature:bytes = overlay.Node;
    pub id: PublicKey,
    pub overlay: Int256,
    pub version: i32,
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct OverlayNodes {
    pub nodes: Vec<OverlayNode>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xe487290e)]
pub struct OverlayNodesBoxed {
    pub nodes: Vec<OverlayNode>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayCertificate {
    /// overlay.emptyCertificate = overlay.Certificate;
    #[tl(id = 0x32dabccf)]
    Empty,
    /// overlay.certificate issued_by:PublicKey expire_at:int max_size:int signature:bytes = overlay.Certificate;
    #[tl(id = 0xe09ed731)]
    Certificate {
        issued_by: PublicKey,
        expire_at: i32,
        max_size: i32,
        signature: Vec<u8>,
    },
    /// overlay.certificateV2 issued_by:PublicKey expire_at:int max_size:int flags:int signature:bytes = overlay.Certificate;
    #[tl(id = 0xb43f9c83)]
    CertificateV2 {
        issued_by: PublicKey,
        expire_at: i32,
        max_size: i32,
        flags: i32,
        signature: Vec<u8>,
    },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayBroadcast {
    /// overlay.unicast data:bytes = overlay.Broadcast;
    #[tl(id = 0x33534e24)]
    Unicast { data: Vec<u8> },
    /// overlay.broadcast src:PublicKey certificate:overlay.Certificate flags:int data:bytes date:int signature:bytes = overlay.Broadcast;
    #[tl(id = 0xb15a2b6b)]
    Broadcast {
        src: PublicKey,
        certificate: OverlayCertificate,
        flags: i32,
        data: Vec<u8>,
        date: i32,
        signature: Vec<u8>,
    },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum FecType {
    /// fec.raptorQ data_size:int symbol_size:int symbols_count:int = fec.Type;
    #[tl(id = 0x8b93a7e0)]
    RaptorQ {
        data_size: i32,
        symbol_size: i32,
        symbols_count: i32,
    },
    /// fec.roundRobin data_size:int symbol_size:int symbols_count:int = fec.Type;
    #[tl(id = 0x32f528e4)]
    RoundRobin {
        data_size: i32,
        symbol_size: i32,
        symbols_count: i32,
    },
    /// fec.online data_size:int symbol_size:int symbols_count:int = fec.Type;
    #[tl(id = 0x0127660c)]
    Online {
        data_size: i32,
        symbol_size: i32,
        symbols_count: i32,
    },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xbad7c36a)]
pub struct OverlayBroadcastFec {
    /// overlay.broadcastFec ... = overlay.Broadcast;
    pub src: PublicKey,
    pub certificate: OverlayCertificate,
    pub data_hash: Int256,
    pub data_size: i32,
    pub flags: i32,
    pub data: Vec<u8>,
    pub seqno: i32,
    pub fec: FecType,
    pub date: i32,
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(
    boxed,
    id = 0xfa374e7c,
    scheme_inline = r##"overlay.broadcast.toSign hash:int256 date:int = overlay.broadcast.ToSign;"##
)]
pub struct OverlayBroadcastToSign {
    pub hash: Int256,
    pub date: i32,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct TonNodeExternalMessage {
    /// tonNode.externalMessage data:bytes = tonNode.ExternalMessage;
    pub data: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x3d1b1867)]
pub struct TonNodeExternalMessageBroadcast {
    /// tonNode.externalMessageBroadcast message:tonNode.externalMessage = tonNode.Broadcast;
    pub message: TonNodeExternalMessage,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x4d9ed329)]
pub struct TonNodeShardPublicOverlayId {
    /// tonNode.shardPublicOverlayId workchain:int shard:long zero_state_file_hash:int256 = tonNode.ShardPublicOverlayId;
    pub workchain: i32,
    pub shard: i64,
    pub zero_state_file_hash: Int256,
}

impl OverlayBroadcast {
    pub fn payload_if_valid(&self, now: i32) -> Option<&[u8]> {
        let Self::Broadcast {
            src: PublicKey::Ed25519 { key },
            data,
            date,
            signature,
            ..
        } = self
        else {
            return None;
        };
        if *date > now.saturating_add(60) || signature.len() != 64 {
            return None;
        }
        let signature = Signature::from_slice(signature).ok()?;
        let public_key = VerifyingKey::from_bytes(&key.0).ok()?;
        let hash = Int256(Sha256::digest(data).into());
        let to_sign = OverlayBroadcastToSign { hash, date: *date };
        public_key
            .verify(&tl_proto::serialize(to_sign), &signature)
            .ok()?;
        Some(data)
    }
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayQuery {
    /// overlay.getRandomPeers peers:overlay.nodes = overlay.Nodes;
    #[tl(id = 0x48ee64ab)]
    GetRandomPeers { peers: OverlayNodes },
    /// overlay.ping = overlay.Pong;
    #[tl(id = 0x690cb481)]
    Ping,
    /// overlay.query overlay:int256 = True;
    #[tl(id = 0xccfd8443)]
    Query { overlay: Int256 },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum DhtQuery {
    /// dht.query node:dht.node = True;
    #[tl(id = 0x7d530769)]
    Query { node: DhtNode },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum DhtValueResult {
    /// dht.valueNotFound nodes:dht.nodes = dht.ValueResult;
    #[tl(id = 0xa2620568)]
    NotFound { nodes: DhtNodes },
    /// dht.valueFound value:dht.value = dht.ValueResult;
    #[tl(id = 0xe6e9fbec)]
    Found { value: DhtValue },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayMessage {
    /// overlay.message overlay:int256 = overlay.Message;
    #[tl(id = 0x75252420)]
    Message { overlay: Int256 },
    /// overlay.unicast data:bytes = overlay.Broadcast;
    #[tl(id = 0x33534e24)]
    Unicast { data: Vec<u8> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tl_proto::{deserialize, serialize};

    #[test]
    fn dht_find_node_has_canonical_constructor() {
        let value = DhtMessage::FindNode {
            key: Int256([7; 32]),
            k: 8,
        };
        let bytes = serialize(value);
        assert_eq!(&bytes[..4], &0x6ce2ce6bu32.to_le_bytes());
        let decoded: DhtMessage = deserialize(&bytes).unwrap();
        assert_eq!(
            decoded,
            DhtMessage::FindNode {
                key: Int256([7; 32]),
                k: 8
            }
        );
    }

    #[test]
    fn packet_contents_roundtrips_optional_channel_fields() {
        let value = PacketContents {
            rand1: vec![1, 2, 3],
            flags: (),
            from: Some(PublicKey::Ed25519 {
                key: Int256([4; 32]),
            }),
            from_short: None,
            message: Some(AdnlMessage::Custom {
                data: vec![9, 8, 7],
            }),
            messages: None,
            address: Some(AddressList {
                addrs: vec![Address::Udp {
                    ip: 0x0100007f,
                    port: 30303,
                }],
                version: 1,
                reinit_date: 2,
                priority: 3,
                expire_at: 4,
            }),
            priority_address: None,
            seqno: Some(11),
            confirm_seqno: Some(10),
            recv_addr_list_version: None,
            recv_priority_addr_list_version: None,
            reinit_date: None,
            dst_reinit_date: None,
            signature: None,
            rand2: vec![5, 6],
        };
        let bytes = serialize(value.clone());
        assert_eq!(&bytes[..4], &0xd142cd89u32.to_le_bytes());
        let decoded: PacketContents = deserialize(&bytes).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn overlay_broadcast_uses_canonical_constructor() {
        let bytes = serialize(OverlayBroadcast::Unicast {
            data: vec![1, 2, 3],
        });
        assert_eq!(&bytes[..4], &0x33534e24u32.to_le_bytes());
        let decoded: OverlayBroadcast = deserialize(&bytes).unwrap();
        assert_eq!(
            decoded,
            OverlayBroadcast::Unicast {
                data: vec![1, 2, 3]
            }
        );
    }

    #[test]
    fn signed_overlay_broadcast_rejects_tampering() {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[13; 32]);
        let data = vec![4, 5, 6];
        let to_sign = OverlayBroadcastToSign {
            hash: Int256(Sha256::digest(&data).into()),
            date: 100,
        };
        let signature = ed25519_dalek::Signer::sign(&signing_key, &serialize(to_sign));
        let mut broadcast = OverlayBroadcast::Broadcast {
            src: PublicKey::Ed25519 {
                key: Int256(signing_key.verifying_key().to_bytes()),
            },
            certificate: OverlayCertificate::Empty,
            flags: 0,
            data,
            date: 100,
            signature: signature.to_bytes().to_vec(),
        };
        assert_eq!(broadcast.payload_if_valid(100), Some([4, 5, 6].as_slice()));
        if let OverlayBroadcast::Broadcast { data, .. } = &mut broadcast {
            data[0] ^= 1;
        }
        assert!(broadcast.payload_if_valid(100).is_none());
    }

    #[test]
    fn external_message_broadcast_uses_canonical_constructor() {
        let bytes = serialize(TonNodeExternalMessageBroadcast {
            message: TonNodeExternalMessage {
                data: vec![0xb5, 0xee, 0x9c, 0x72],
            },
        });
        assert_eq!(&bytes[..4], &0x3d1b1867u32.to_le_bytes());
        let decoded: TonNodeExternalMessageBroadcast = deserialize(&bytes).unwrap();
        assert_eq!(decoded.message.data, vec![0xb5, 0xee, 0x9c, 0x72]);
    }
}
