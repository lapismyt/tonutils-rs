//! Overlay wire types, split out of [`super::network`].
//!
//! The `overlay.*` namespace types no longer fit next to the ADNL,
//! DHT, and QUIC values in `network.rs` without pushing that file
//! past the repository's line limit, so they live here and are
//! re-exported from `network` to keep every existing import path -
//! `tonutils_tl::tl::network::OverlayQuery` and friends - working.
//!
//! Constructor ids for the types without an explicit `#` in the
//! scheme are CRC32 of the definition line with parentheses removed,
//! the ecosystem normalization; [`OverlayQuery::GetRandomPeersV2`]
//! is additionally pinned by a live capture.

use derivative::Derivative;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use tl_proto::{TlRead, TlWrite};

use super::common::Int256;
use super::network::{AdnlIdShort, PublicKey};

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
pub struct OverlayNodeV2 {
    /// overlay.nodeV2 id:PublicKey overlay:int256 flags:int version:int signature:bytes certificate:overlay.MemberCertificate = overlay.NodeV2;
    pub id: PublicKey,
    pub overlay: Int256,
    pub flags: i32,
    pub version: i32,
    pub signature: Vec<u8>,
    pub certificate: OverlayMemberCertificate,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x03d8a8e1)]
pub struct OverlayNodeToSign {
    /// overlay.node.toSign id:adnl.id.short overlay:int256 version:int = overlay.node.ToSign;
    pub id: AdnlIdShort,
    pub overlay: Int256,
    pub version: i32,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayMemberCertificate {
    /// overlay.emptyMemberCertificate = overlay.MemberCertificate;
    #[tl(id = 0xc02441e2)]
    Empty,
    /// overlay.memberCertificate issued_by:PublicKey flags:int slot:int expire_at:int signature:bytes = overlay.MemberCertificate;
    #[tl(id = 0xc2008c59)]
    Certificate {
        issued_by: PublicKey,
        flags: i32,
        slot: i32,
        expire_at: i32,
        signature: Vec<u8>,
    },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
pub struct OverlayMessageExtra {
    #[tl(flags)]
    pub flags: (),
    #[tl(flags_bit = "flags.0")]
    pub certificate: Option<OverlayMemberCertificate>,
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
pub struct OverlayNodesV2 {
    pub nodes: Vec<OverlayNodeV2>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xe4071842)]
pub struct OverlayNodesV2Boxed {
    pub nodes: Vec<OverlayNodeV2>,
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
    /// overlay.broadcastFec src:PublicKey certificate:overlay.Certificate data_hash:int256 data_size:int flags:int data:bytes seqno:int fec:fec.Type date:int signature:bytes = overlay.Broadcast;
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

/// overlay.pong = overlay.Pong;
///
/// The empty answer to [`OverlayQuery::Ping`]; peers that reach this node
/// through an overlay broadcast channel expect this exact constructor.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x67700804)]
pub struct OverlayPong;

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayQuery {
    /// overlay.getRandomPeers peers:overlay.nodes = overlay.Nodes;
    #[tl(id = 0x48ee64ab)]
    GetRandomPeers { peers: OverlayNodes },
    /// overlay.getRandomPeersV2 peers:overlay.nodesV2 = overlay.NodesV2;
    ///
    /// The id `0xa58e7ecc` was captured from the live wire; it matches
    /// the CRC32 of the definition line under the ecosystem
    /// normalization, and upstream dispatches it in
    /// `OverlayImpl::process_query` for every non-private overlay.
    #[tl(id = 0xa58e7ecc)]
    GetRandomPeersV2 { peers: OverlayNodesV2 },
    /// overlay.ping = overlay.Pong;
    #[tl(id = 0x690cb481)]
    Ping,
    /// tonNode.getCapabilities = tonNode.Capabilities;
    ///
    /// Dispatched by `validator/full-node-queries.hpp` on the mainnet
    /// full-node overlays, so it arrives wrapped in `overlay.query` exactly
    /// like `overlay.ping` does.
    #[tl(id = 0xdee618f8)]
    GetCapabilities,
    /// overlay.query overlay:int256 = True;
    #[tl(id = 0xccfd8443)]
    Query { overlay: Int256 },
    /// overlay.queryWithExtra overlay:int256 extra:overlay.messageExtra = True;
    #[tl(id = 0x94ffc3e9)]
    QueryWithExtra {
        overlay: Int256,
        extra: OverlayMessageExtra,
    },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed)]
pub enum OverlayMessage {
    /// overlay.message overlay:int256 = overlay.Message;
    #[tl(id = 0x75252420)]
    Message { overlay: Int256 },
    /// overlay.messageWithExtra overlay:int256 extra:overlay.messageExtra = overlay.Message;
    #[tl(id = 0xa232233d)]
    MessageWithExtra {
        overlay: Int256,
        extra: OverlayMessageExtra,
    },
    /// overlay.unicast data:bytes = overlay.Broadcast;
    #[tl(id = 0x33534e24)]
    Unicast { data: Vec<u8> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tl_proto::{deserialize, serialize};

    #[test]
    fn overlay_query_wrapper_frames_inner_query() {
        let overlay = Int256([9; 32]);
        let wrapper = OverlayQuery::Query {
            overlay: overlay.clone(),
        };
        let bytes = serialize(wrapper.clone());
        // `overlay.query overlay:int256 = True;`
        assert_eq!(&bytes[..4], &[0x43, 0x84, 0xfd, 0xcc]);
        assert_eq!(&bytes[4..], &[9; 32]);
        assert_eq!(deserialize::<OverlayQuery>(&bytes).unwrap(), wrapper);

        let with_extra = OverlayQuery::QueryWithExtra {
            overlay: overlay.clone(),
            extra: OverlayMessageExtra {
                flags: (),
                certificate: None,
            },
        };
        let bytes = serialize(with_extra.clone());
        // `overlay.queryWithExtra overlay:int256 extra:overlay.messageExtra = True;`
        assert_eq!(&bytes[..4], &[0xe9, 0xc3, 0xff, 0x94]);
        assert_eq!(
            deserialize::<OverlayQuery>(&bytes).unwrap(),
            with_extra,
            "extra is a constructor-typed field and must serialize bare"
        );

        // Upstream framing: header bytes followed by the raw inner query.
        let mut framed = serialize(OverlayQuery::Query { overlay });
        framed.extend_from_slice(&serialize(OverlayQuery::Ping));
        assert_eq!(&framed[..4], &[0x43, 0x84, 0xfd, 0xcc]);
        assert_eq!(&framed[36..], &[0x81, 0xb4, 0x0c, 0x69]);
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
    fn overlay_v2_types_use_canonical_constructors() {
        // Every V2 id is the CRC32 of its definition line with
        // parentheses removed; getRandomPeersV2's additionally matches
        // the id captured from the live wire.
        let node = OverlayNodeV2 {
            id: PublicKey::Ed25519 {
                key: Int256([1; 32]),
            },
            overlay: Int256([2; 32]),
            flags: 0,
            version: 3,
            signature: vec![4; 64],
            certificate: OverlayMemberCertificate::Empty,
        };
        let answer = OverlayNodesV2Boxed {
            nodes: vec![node.clone()],
        };
        let bytes = serialize(answer.clone());
        assert_eq!(&bytes[..4], &0xe4071842u32.to_le_bytes());
        assert_eq!(
            deserialize::<OverlayNodesV2Boxed>(&bytes).unwrap(),
            answer,
            "nodeV2 records are concrete vector elements and serialize bare"
        );

        let query = OverlayQuery::GetRandomPeersV2 {
            peers: OverlayNodesV2 { nodes: vec![node] },
        };
        let bytes = serialize(query.clone());
        assert_eq!(&bytes[..4], &0xa58e7eccu32.to_le_bytes());
        assert_eq!(deserialize::<OverlayQuery>(&bytes).unwrap(), query);
    }
}
