//! Full-node overlay queries that are not part of `overlay.*`.
//!
//! The mainnet basechain shard overlay is a *full-node* overlay, so the
//! queries delivered on it are dispatched by
//! `validator/full-node-queries.hpp::FullNodeQueries::handle_query`, not by
//! `OverlayImpl::process_query`.  Anything that is not a known full-node
//! constructor falls through to the generic
//! `process_query(T, adnl::AdnlNodeIdShort, QuerySource)` overload at
//! `full-node-queries.hpp:237`, which answers `unknown query`, so an
//! unanswered constructor here is indistinguishable from a broken node to the
//! peer that asked.
//!
//! Only the constructors this SDK can answer honestly live here; the rest are
//! recorded in `TODO.md`.

use derivative::Derivative;
use tl_proto::{TlRead, TlWrite};

/// `tonNode.getCapabilities`, the capability probe every mainnet full node
/// sends to a peer it has just learned about.
///
/// The definition line `tonNode.getCapabilities = tonNode.Capabilities;`
/// carries no explicit `#`, so its constructor id is derived by the upstream
/// TL compiler and is not present in the checked-in schema snapshot.  The
/// value used here, `0xdee618f8`, is the id captured from the live wire by
/// `crates/tonutils-mempool/tests/live.rs`, where it accounts for the
/// majority of otherwise unhandled inbound overlay queries.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xdee618f8)]
pub struct TonNodeGetCapabilities;

/// `tonNode.capabilities#f5bf60c0 version_major:int version_minor:int flags:# = tonNode.Capabilities;`
///
/// The fields describe the *full-node protocol*, not validator status:
/// `version_major`/`version_minor` are `FullNode::PROTO_VERSION_MAJOR` and
/// `FullNode::PROTO_VERSION_MINOR` (`validator/full-node.h:186`), and `flags`
/// is the optional capability bitmask.  Upstream answers with a literal `0`
/// for `flags` without consulting any node state, so reporting no optional
/// capabilities is accurate for this SDK.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xf5bf60c0)]
pub struct TonNodeCapabilities {
    /// `FullNode::PROTO_VERSION_MAJOR`.
    pub version_major: i32,
    /// `FullNode::PROTO_VERSION_MINOR`.
    pub version_minor: i32,
    /// Optional capability bits; `0` means none are offered.
    pub flags: i32,
}

/// `FullNode::PROTO_VERSION_MAJOR` (`validator/full-node.h:186`).
pub const FULL_NODE_PROTO_VERSION_MAJOR: i32 = 3;
/// `FullNode::PROTO_VERSION_MINOR` (`validator/full-node.h:187`).
pub const FULL_NODE_PROTO_VERSION_MINOR: i32 = 2;

impl TonNodeCapabilities {
    /// Capabilities a node answering with upstream's own defaults would send.
    #[must_use]
    pub fn current() -> Self {
        Self {
            version_major: FULL_NODE_PROTO_VERSION_MAJOR,
            version_minor: FULL_NODE_PROTO_VERSION_MINOR,
            flags: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_serialize_with_the_upstream_constructor() {
        let bytes = tl_proto::serialize(TonNodeCapabilities::current());
        let mut expected = 0xf5bf60c0u32.to_le_bytes().to_vec();
        expected.extend_from_slice(&3i32.to_le_bytes());
        expected.extend_from_slice(&2i32.to_le_bytes());
        expected.extend_from_slice(&0i32.to_le_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn get_capabilities_decodes_from_the_wire_id() {
        let bytes = 0xdee618f8u32.to_le_bytes().to_vec();
        assert_eq!(
            tl_proto::deserialize::<TonNodeGetCapabilities>(&bytes).expect("wire id must decode"),
            TonNodeGetCapabilities
        );
    }
}
