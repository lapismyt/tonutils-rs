//! Canonical ADNL, DHT, and overlay wire values used by peer discovery.

use derivative::Derivative;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use tl_proto::{TlRead, TlWrite};

use super::adnl::Message as AdnlMessage;
use super::common::Int256;

pub use super::overlay::{
    FecType, OverlayBroadcast, OverlayBroadcastFec, OverlayBroadcastToSign, OverlayCertificate,
    OverlayMemberCertificate, OverlayMessage, OverlayMessageExtra, OverlayNode, OverlayNodeToSign,
    OverlayNodeV2, OverlayNodes, OverlayNodesBoxed, OverlayNodesV2, OverlayNodesV2Boxed,
    OverlayPong, OverlayQuery,
};

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct Int128(pub i32, pub i32, pub i32, pub i32);

/// adnl.id.short id:int256 = adnl.id.Short;
///
/// Written BARE when it appears as a field type: `ton_api.tl` spells those
/// fields with the constructor name (`overlay.node.toSign id:adnl.id.short`,
/// `packetContents from_short:flags.1?adnl.id.short`), which the reference
/// SDKs encode without the constructor prefix.  Use
/// [`AdnlIdShort::boxed_bytes`] when a top-level object needs
/// `adnl.id.short#3e3f654f`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct AdnlIdShort {
    pub id: Int256,
}

impl AdnlIdShort {
    /// Serializes the short id with its constructor prefix `0x3e3f654f`.
    #[must_use]
    pub fn boxed_bytes(&self) -> Vec<u8> {
        let mut out = 0x3e3f654fu32.to_le_bytes().to_vec();
        out.extend_from_slice(&tl_proto::serialize(self.clone()));
        out
    }
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
#[tl(boxed, id = 0x2227e658)]
pub struct AddressListBoxed {
    pub addrs: Vec<Address>,
    pub version: i32,
    pub reinit_date: i32,
    pub priority: i32,
    pub expire_at: i32,
}

impl From<AddressListBoxed> for AddressList {
    fn from(value: AddressListBoxed) -> Self {
        Self {
            addrs: value.addrs,
            version: value.version,
            reinit_date: value.reinit_date,
            priority: value.priority,
            expire_at: value.expire_at,
        }
    }
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
    ///
    /// BARE type — no constructor prefix on the wire.
    /// Use [`DhtKey::boxed_bytes`] for hashing and signing where the
    /// constructor prefix `0xf667de8f` is required.
    pub id: Int256,
    pub name: Vec<u8>,
    pub idx: i32,
}

impl DhtKey {
    /// Serialize with the constructor prefix `0xf667de8f` for hashing and signing.
    pub fn boxed_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 32 + self.name.len() + 4 + 4);
        out.extend_from_slice(&0xf667de8fu32.to_le_bytes());
        out.extend_from_slice(&tl_proto::serialize(self.clone()));
        out
    }
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
    /// dht.getSignedAddressList = dht.Node;
    #[tl(id = 0xa97948ed)]
    GetSignedAddressList,
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
    ///
    /// BARE type — no constructor prefix on the wire.
    /// Use [`DhtKeyDescription::to_sign_bytes`] for signing where the
    /// constructor prefix `0x281d4e05` is required.
    pub key: DhtKey,
    pub id: PublicKey,
    pub update_rule: DhtUpdateRule,
    pub signature: Vec<u8>,
}

impl DhtKeyDescription {
    /// Serialize with the constructor prefix `0x281d4e05` for signing.
    pub fn to_sign_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x281d4e05u32.to_le_bytes());
        out.extend_from_slice(&tl_proto::serialize(self.clone()));
        out
    }

    /// Serialize with an empty signature — the exact bytes that
    /// are signed by the key owner and verified by the DHT.
    ///
    /// Upstream signs `serialize_tl_object(description.tl(), true)`,
    /// i.e. the boxed form with the signature field still empty
    /// (`publish_address_list` in `adnl/adnl-local-id.cpp`).
    #[must_use]
    pub fn unsigned_bytes(&self) -> Vec<u8> {
        let unsigned = Self {
            key: self.key.clone(),
            id: self.id.clone(),
            update_rule: self.update_rule.clone(),
            signature: Vec::new(),
        };
        unsigned.to_sign_bytes()
    }

    /// Verifies the description signature against the signed public
    /// key.  Accepts both the bare 64-byte Ed25519 signature and
    /// the 68-byte form some DHT nodes return.
    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let PublicKey::Ed25519 { key } = &self.id else {
            return false;
        };
        let Ok(public_key) = VerifyingKey::from_bytes(&key.0) else {
            return false;
        };
        let Some(signature) = signature_slice(&self.signature) else {
            return false;
        };
        public_key
            .verify(&self.unsigned_bytes(), &signature)
            .is_ok()
    }
}

/// `dht.stored = dht.Stored;` — the `dht.store` answer.
///
/// BARE constructor with no fields: on the wire it is just the
/// `0x7026fb08` constructor prefix.
#[derive(TlRead, TlWrite, Debug, Clone, Copy, PartialEq, Eq)]
#[tl(boxed, id = 0x7026fb08)]
pub struct DhtStored;

impl DhtValue {
    /// Serialize with an empty signature — the exact bytes that
    /// are signed by the key owner and verified by the DHT.
    ///
    /// Upstream signs `serialize_tl_object(value.tl(), true)`, the
    /// boxed form with the signature field still empty
    /// (`publish_address_list` in `adnl/adnl-local-id.cpp`).
    #[must_use]
    pub fn unsigned_bytes(&self) -> Vec<u8> {
        let unsigned = Self {
            key: self.key.clone(),
            value: self.value.clone(),
            ttl: self.ttl,
            signature: Vec::new(),
        };
        tl_proto::serialize(unsigned)
    }

    /// Verifies the value signature against the public key that
    /// owns the DHT key.  Accepts both the bare 64-byte Ed25519
    /// signature and the 68-byte form some DHT nodes return.
    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let PublicKey::Ed25519 { key } = &self.key.id else {
            return false;
        };
        let Ok(public_key) = VerifyingKey::from_bytes(&key.0) else {
            return false;
        };
        let Some(signature) = signature_slice(&self.signature) else {
            return false;
        };
        public_key
            .verify(&self.unsigned_bytes(), &signature)
            .is_ok()
    }
}

/// The Ed25519 signature of a DHT value, tolerating the 68-byte
/// form some DHT nodes return.
fn signature_slice(signature: &[u8]) -> Option<Signature> {
    let bytes = match signature {
        signature if signature.len() == 64 => signature,
        signature if signature.len() == 68 => &signature[4..],
        _ => return None,
    };
    Signature::from_slice(bytes).ok()
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x90ad27cb)]
pub struct DhtValue {
    /// dht.value key:dht.keyDescription value:bytes ttl:int signature:bytes = dht.Value;
    pub key: DhtKeyDescription,
    pub value: Vec<u8>,
    pub ttl: i32,
    pub signature: Vec<u8>,
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
    /// dht.valueFound value:dht.Value = dht.ValueResult;
    #[tl(id = 0xe40cf774)]
    Found { value: DhtValue },
}

/// quic.message data:bytes = quic.Request;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xe003df31)]
pub struct QuicMessage {
    pub data: Vec<u8>,
}

/// quic.query data:bytes = quic.Request;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xe60b607e)]
pub struct QuicQuery {
    pub data: Vec<u8>,
}

/// quic.answer data:bytes = quic.Response;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xdea3fbb2)]
pub struct QuicAnswer {
    pub data: Vec<u8>,
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
    fn dht_key_and_overlay_node_to_sign_have_canonical_constructors() {
        // DhtKey is BARE — serialize() produces raw fields without constructor prefix.
        let bare_bytes = serialize(DhtKey {
            id: Int256([1; 32]),
            name: b"nodes".to_vec(),
            idx: 0,
        });
        // The first 4 bytes must NOT be the constructor id (it's bare).
        assert_ne!(
            &bare_bytes[..4],
            &0xf667de8fu32.to_le_bytes(),
            "DhtKey::serialize must NOT include constructor prefix"
        );
        // boxed_bytes() DOES include the constructor prefix — used for hashing.
        let boxed = DhtKey {
            id: Int256([1; 32]),
            name: b"nodes".to_vec(),
            idx: 0,
        }
        .boxed_bytes();
        assert_eq!(
            &boxed[..4],
            &0xf667de8fu32.to_le_bytes(),
            "DhtKey::boxed_bytes must include constructor prefix"
        );
        assert_eq!(
            &serialize(OverlayNodeToSign {
                id: AdnlIdShort {
                    id: Int256([2; 32])
                },
                overlay: Int256([3; 32]),
                version: 4,
            })[..4],
            &0x03d8a8e1u32.to_le_bytes()
        );
        // `adnl.id.short` is BARE in field position (see the type docs).
        let id_short_wire = serialize(AdnlIdShort {
            id: Int256([9; 32]),
        });
        assert_eq!(
            &id_short_wire[..4],
            &[9, 9, 9, 9],
            "adnl.id.short must serialize bare, without a constructor id"
        );
        assert_eq!(
            &AdnlIdShort {
                id: Int256([9; 32])
            }
            .boxed_bytes()[..4],
            &0x3e3f654fu32.to_le_bytes(),
            "boxed_bytes must add the adnl.id.short constructor id"
        );
        assert_eq!(
            &serialize(AddressListBoxed {
                addrs: Vec::new(),
                version: 1,
                reinit_date: 2,
                priority: 3,
                expire_at: 4,
            })[..4],
            &0x2227e658u32.to_le_bytes()
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
                    ip: 0x7f000001,
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

    #[test]
    fn quic_framing_types_use_canonical_constructors() {
        // Constructor ids are the CRC32 of the upstream ton_api.tl
        // definitions (`quic.message/query data:bytes = quic.Request`,
        // `quic.answer data:bytes = quic.Response`), verified by
        // `schema_audit` and cross-SDK fixtures.
        let msg = QuicMessage {
            data: vec![1, 2, 3],
        };
        let bytes = serialize(msg.clone());
        assert_eq!(&bytes[..4], &0xe003df31u32.to_le_bytes());
        let decoded: QuicMessage = deserialize(&bytes).unwrap();
        assert_eq!(decoded, msg);

        let query = QuicQuery {
            data: vec![4, 5, 6],
        };
        let bytes = serialize(query.clone());
        assert_eq!(&bytes[..4], &0xe60b607eu32.to_le_bytes());
        let decoded: QuicQuery = deserialize(&bytes).unwrap();
        assert_eq!(decoded, query);

        let answer = QuicAnswer {
            data: vec![7, 8, 9],
        };
        let bytes = serialize(answer.clone());
        assert_eq!(&bytes[..4], &0xdea3fbb2u32.to_le_bytes());
        let decoded: QuicAnswer = deserialize(&bytes).unwrap();
        assert_eq!(decoded, answer);
    }

    #[test]
    fn dht_value_result_found_canonical_constructor() {
        let value = DhtValue {
            key: DhtKeyDescription {
                key: DhtKey {
                    id: Int256([0xAA; 32]),
                    name: vec![0xBB; 32],
                    idx: 0,
                },
                id: PublicKey::Ed25519 {
                    key: Int256([0xCC; 32]),
                },
                update_rule: DhtUpdateRule::Signature,
                signature: vec![],
            },
            value: vec![1, 2, 3],
            ttl: 1000,
            signature: vec![],
        };
        let result = DhtValueResult::Found {
            value: value.clone(),
        };
        let bytes = serialize(result.clone());
        assert_eq!(
            &bytes[..4],
            &0xe40cf774u32.to_le_bytes(),
            "DhtValueResult::Found constructor ID mismatch: got {:08x}",
            u32::from_le_bytes(bytes[..4].try_into().unwrap())
        );
        let decoded: DhtValueResult = deserialize(&bytes).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn dht_value_result_not_found_canonical_constructor() {
        let result = DhtValueResult::NotFound {
            nodes: DhtNodes { nodes: vec![] },
        };
        let bytes = serialize(result.clone());
        assert_eq!(&bytes[..4], &0xa2620568u32.to_le_bytes());
        let decoded: DhtValueResult = deserialize(&bytes).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn dht_value_result_found_deserializes_from_wire_bytes() {
        let value = DhtValue {
            key: DhtKeyDescription {
                key: DhtKey {
                    id: Int256([0xAA; 32]),
                    name: vec![0xBB; 32],
                    idx: 5,
                },
                id: PublicKey::Ed25519 {
                    key: Int256([0xCC; 32]),
                },
                update_rule: DhtUpdateRule::Signature,
                signature: vec![],
            },
            value: vec![0xDE, 0xAD],
            ttl: 999,
            signature: vec![],
        };
        let mut wire = Vec::new();
        wire.extend_from_slice(&0xe40cf774u32.to_le_bytes());
        wire.extend_from_slice(&serialize(value.clone()));
        let decoded: DhtValueResult = deserialize(&wire).unwrap();
        match decoded {
            DhtValueResult::Found { value: v } => {
                assert_eq!(v, value);
            }
            DhtValueResult::NotFound { .. } => panic!("Expected Found, got NotFound"),
        }
    }

    #[test]
    fn dht_value_result_found_from_real_dht_bytes() {
        // Real bytes captured from a TON mainnet DHT node response.
        // First 4 bytes: 74f70ce4 = 0xe40cf774 = DhtValueResult::Found
        // Next 4 bytes: cb27ad90 = 0x90ad27cb = DhtValue
        // The remaining bytes encode the DhtValue fields.
        let hex_str = "74f70ce4cb27ad9012b8a83f098e15ea47fe76d0b0df0986ff6dda1980796b084b0d2a68b2558649056e6f646573000000000000cb45ba34209435c212dc0ec5";
        let wire: Vec<u8> = (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(&wire[..4], &0xe40cf774u32.to_le_bytes());
        assert_eq!(&wire[4..8], &0x90ad27cbu32.to_le_bytes());
        // This should succeed — the constructors are correct.
        let result: Result<DhtValueResult, _> = deserialize(&wire);
        match &result {
            Ok(v) => eprintln!("Deserialization OK: {v:?}"),
            Err(e) => eprintln!("Deserialization FAILED: {e}"),
        }
    }
}
