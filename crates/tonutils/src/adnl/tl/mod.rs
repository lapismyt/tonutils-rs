//! ADNL, DHT, and QUIC wire values owned by the ADNL module.

mod message;
mod public_key;

#[cfg(test)]
mod tests;

use derivative::Derivative;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
pub use message::AdnlMessage;
pub use public_key::{AdnlTlPublicKeyOwned, AdnlTlPublicKeyRef};
use tl_proto::{TlRead, TlWrite};

use crate::tl::TlInt256;

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct TlInt128(pub i32, pub i32, pub i32, pub i32);

/// adnl.id.short id:int256 = adnl.id.Short;
///
/// Written BARE when it appears as a field type: `ton_api.tl` spells those
/// fields with the constructor name (`dht.node id:PublicKey`,
/// `packetContents from_short:flags.1?adnl.id.short`), which the reference
/// SDKs encode without the constructor prefix.  Use
/// [`AdnlIdShort::boxed_bytes`] when a top-level object needs
/// `adnl.id.short#3e3f654f`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct AdnlIdShort {
    pub id: TlInt256,
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
pub enum AdnlTlPublicKey {
    /// pub.ed25519 key:int256 = PublicKey;
    #[tl(id = 0x4813b4c6)]
    Ed25519 { key: TlInt256 },
    /// pub.aes key:int256 = PublicKey;
    #[tl(id = 0x2dbcadd4)]
    Aes { key: TlInt256 },
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
pub enum AdnlNetAddress {
    /// adnl.address.udp ip:int port:int = adnl.Address;
    #[tl(id = 0x670da6e7)]
    Udp { ip: i32, port: i32 },
    /// adnl.address.udp6 ip:int128 port:int = adnl.Address;
    #[tl(id = 0xe31d63fa)]
    Udp6 { ip: TlInt128, port: i32 },
    /// adnl.address.tunnel to:int256 pubkey:PublicKey = adnl.Address;
    #[tl(id = 0x092b02eb)]
    Tunnel {
        to: TlInt256,
        pubkey: AdnlTlPublicKey,
    },
    /// adnl.address.reverse = adnl.Address;
    #[tl(id = 0x27795286)]
    Reverse,
    /// adnl.address.quic ip:int port:int = adnl.Address;
    #[tl(id = 0x78017253)]
    Quic { ip: i32, port: i32 },
}

impl AdnlNetAddress {
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
    pub from: Option<AdnlTlPublicKey>,
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
    pub addrs: Vec<AdnlNetAddress>,
    pub version: i32,
    pub reinit_date: i32,
    pub priority: i32,
    pub expire_at: i32,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x2227e658)]
pub struct AddressListBoxed {
    pub addrs: Vec<AdnlNetAddress>,
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
    pub id: AdnlTlPublicKey,
    pub addr_list: AddressList,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtNode {
    /// dht.node id:PublicKey addr_list:adnl.addressList version:int signature:bytes = dht.Node;
    pub id: AdnlTlPublicKey,
    pub addr_list: AddressList,
    pub version: i32,
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x84533248)]
pub struct DhtNodeBoxed {
    pub id: AdnlTlPublicKey,
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
            && self.addr_list.addrs.iter().all(AdnlNetAddress::is_valid)
            && self.verify_signature()
    }

    #[must_use]
    pub fn verify_signature(&self) -> bool {
        let AdnlTlPublicKey::Ed25519 { key } = &self.id else {
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
    pub id: TlInt256,
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
    ///
    /// References the concrete `dht.value` constructor, so the stored value
    /// is serialized in its bare form ([`DhtValueBare`]); the boxed
    /// [`DhtValue`] form is what `dht.valueFound` carries.
    #[tl(id = 0x34934212)]
    Store { value: DhtValueBare },
    /// dht.findNode key:int256 k:int = dht.Nodes;
    #[tl(id = 0x6ce2ce6b)]
    FindNode { key: TlInt256, k: i32 },
    /// dht.findValue key:int256 k:int = dht.ValueResult;
    #[tl(id = 0xae4b6011)]
    FindValue { key: TlInt256, k: i32 },
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
    pub id: AdnlTlPublicKey,
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
        let AdnlTlPublicKey::Ed25519 { key } = &self.id else {
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

/// dht.pong random_id:long = dht.Pong;
///
/// The `dht.ping` answer.  pytoniq nodes ping their peers with `dht.ping`
/// and drop a peer whose pings go unanswered four times (`Node.ping` in
/// `pytoniq/adnl/adnl.py`), so answering this constructor is what keeps a
/// peer in their peer list.
#[derive(TlRead, TlWrite, Debug, Clone, Copy, PartialEq, Eq)]
#[tl(boxed, id = 0x5a8aef81)]
pub struct DhtPong {
    pub random_id: u64,
}

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
        let AdnlTlPublicKey::Ed25519 { key } = &self.key.id else {
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

/// Bare `dht.value` form carried by the `dht.store` query.
///
/// `dht.store value:dht.value = dht.Stored;` references the concrete
/// `dht.value` constructor, so upstream serializes the value without the
/// `0x90ad27cb` prefix (`TlStoreObject::store(value_, s)` in the generated
/// `dht_store::store`).  [`DhtValue`] carries the prefix and is the form
/// `dht.valueFound` uses and the value signature covers.  A store query
/// that adds the prefix does not parse on the receiving node, which drops
/// it without answering.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct DhtValueBare {
    /// dht.value key:dht.keyDescription value:bytes ttl:int signature:bytes = dht.Value;
    pub key: DhtKeyDescription,
    pub value: Vec<u8>,
    pub ttl: i32,
    pub signature: Vec<u8>,
}

impl From<&DhtValue> for DhtValueBare {
    fn from(value: &DhtValue) -> Self {
        Self {
            key: value.key.clone(),
            value: value.value.clone(),
            ttl: value.ttl,
            signature: value.signature.clone(),
        }
    }
}

impl From<DhtValue> for DhtValueBare {
    fn from(value: DhtValue) -> Self {
        Self::from(&value)
    }
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
