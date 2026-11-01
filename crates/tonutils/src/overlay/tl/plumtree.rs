//! Plumtree broadcast, control, repair, and statistics wire types.
//!
//! Constructor ids are the CRC32 of the definition line after removing
//! parentheses, matching the TON TL generator convention.

use derivative::Derivative;
use tl_proto::{Bare, TlRead, TlWrite};

use crate::{
    adnl::tl::AdnlTlPublicKey as TlPublicKey,
    overlay::tl::OverlayCertificate,
    tl::common::{TlInt256 as Int256, TlString},
};

/// One TL `double` field.
///
/// `tl-proto` 0.5's `f64` writer is not the inverse of its reader, so the
/// wire type stores raw IEEE-754 bits and converts explicitly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TlDouble(u64);

impl TlDouble {
    #[must_use]
    pub const fn from_f64(value: f64) -> Self {
        Self(value.to_bits())
    }

    #[must_use]
    pub const fn to_f64(self) -> f64 {
        f64::from_bits(self.0)
    }

    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0
    }
}

impl From<f64> for TlDouble {
    fn from(value: f64) -> Self {
        Self::from_f64(value)
    }
}

impl From<TlDouble> for f64 {
    fn from(value: TlDouble) -> Self {
        value.to_f64()
    }
}

impl TlWrite for TlDouble {
    type Repr = Bare;

    fn max_size_hint(&self) -> usize {
        std::mem::size_of::<u64>()
    }

    fn write_to<P: tl_proto::TlPacket>(&self, packet: &mut P) {
        packet.write_u64(self.0);
    }
}

impl<'a> TlRead<'a> for TlDouble {
    type Repr = Bare;

    fn read_from(packet: &mut &'a [u8]) -> tl_proto::TlResult<Self> {
        u64::read_from(packet).map(Self)
    }
}

/// `tonNode.shardId workchain:int shard:long = tonNode.ShardId`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x28fa0877)]
pub struct TonNodeShardId {
    pub workchain: i32,
    pub shard: i64,
}

/// `overlay.broadcastPlumtreeSimple`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x3eccd26d)]
pub struct OverlayBroadcastPlumtreeSimple {
    pub flags: i32,
    pub timestamp: TlDouble,
    pub src: TlPublicKey,
    pub certificate: OverlayCertificate,
    pub broadcast_id: Int256,
    pub tree_index: i32,
    pub data: Vec<u8>,
    pub signature: Vec<u8>,
}

/// `overlay.broadcastPlumtreeFec`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x8cba5fcf)]
pub struct OverlayBroadcastPlumtreeFec {
    pub flags: i32,
    pub timestamp: TlDouble,
    pub src: TlPublicKey,
    pub certificate: OverlayCertificate,
    pub full_data_hash: Int256,
    pub full_data_size: i32,
    pub part_index: i32,
    pub tree_index: i32,
    pub data: Vec<u8>,
    pub signature: Vec<u8>,
}

/// `overlay.broadcastPlumtreeIHave`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x6150c07b)]
pub struct OverlayBroadcastPlumtreeIHave {
    pub broadcast_id: Int256,
    pub timestamp: TlDouble,
    pub part_index: i32,
    pub tree_index: i32,
    pub src: TlPublicKey,
    pub certificate: OverlayCertificate,
    pub payload_timestamp: TlDouble,
    pub data_size: i32,
    pub data_hash: Int256,
    pub signature: Vec<u8>,
}

/// `overlay.broadcastPlumtreePrune`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xf0f73196)]
pub struct OverlayBroadcastPlumtreePrune {
    pub broadcast_id: Int256,
    pub timestamp: TlDouble,
    pub part_index: i32,
    pub tree_index: i32,
}

/// `overlay.broadcastPlumtreeUseful`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xaf825e83)]
pub struct OverlayBroadcastPlumtreeUseful {
    pub broadcast_id: Int256,
    pub timestamp: TlDouble,
    pub part_index: i32,
    pub tree_index: i32,
}

/// `overlay.repairPlumtreePart`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x7bd0590d)]
pub struct OverlayRepairPlumtreePart {
    pub broadcast_id: Int256,
    pub timestamp: TlDouble,
    pub part_index: i32,
    pub tree_index: i32,
}

/// `overlay.broadcastNotFound`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x95863624)]
pub struct OverlayBroadcastNotFound;

/// `overlay.plumtreeStatsRecord`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xe5056a2e)]
pub struct OverlayPlumtreeStatsRecord {
    pub src: Int256,
    pub epoch: i32,
    pub tree_index: i32,
    pub eager_0: Vec<i64>,
    pub eager_index: Vec<i64>,
    pub delivered_bcsts: i32,
    pub parts_in_p99_bcsts: i32,
    pub useful_avg_ms: i32,
    pub useful_max_ms: i32,
    pub useful_p99_ms: i32,
}

/// `overlay.plumtreeStatsPush`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x22dfaa6e)]
pub struct OverlayPlumtreeStatsPush {
    pub record: OverlayPlumtreeStatsRecord,
}

/// `overlay.plumtreeStatsExchange`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x741db2e9)]
pub struct OverlayPlumtreeStatsExchange {
    pub overlay: Int256,
    pub overlay_type: TlString,
    pub shard: TonNodeShardId,
    pub records: Vec<OverlayPlumtreeStatsRecord>,
}

/// `overlay.plumtreeStatsDump`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xd7442202)]
pub struct OverlayPlumtreeStatsDump {
    pub overlay: Int256,
    pub overlay_type: TlString,
    pub shard: TonNodeShardId,
    pub received_from: Int256,
    pub received_at: TlDouble,
    pub records: Vec<OverlayPlumtreeStatsRecord>,
}

/// `overlay.certificateId overlay_id:int256 node:int256 expire_at:int max_size:int = overlay.CertificateId`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x8fae60b9)]
pub struct OverlayCertificateId {
    pub overlay_id: Int256,
    pub node: Int256,
    pub expire_at: i32,
    pub max_size: i32,
}

/// `overlay.certificateIdV2 overlay_id:int256 node:int256 expire_at:int max_size:int flags:int = overlay.CertificateId`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xfc6cd2a7)]
pub struct OverlayCertificateIdV2 {
    pub overlay_id: Int256,
    pub node: Int256,
    pub expire_at: i32,
    pub max_size: i32,
    pub flags: i32,
}

/// Identity preimage of a Plumtree FEC broadcast.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x06a25034)]
pub struct OverlayBroadcastPlumtreeFecId {
    pub flags: i32,
    pub full_data_hash: Int256,
    pub full_data_size: i32,
    pub part_size: i32,
}

/// Signed preimage of a Plumtree simple payload.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0x38f3338e)]
pub struct OverlayBroadcastPlumtreeSimpleToSign {
    pub id: Int256,
    pub timestamp: TlDouble,
    pub tree_index: i32,
    pub data_size: i32,
    pub data_hash: Int256,
}

/// Signed preimage of one Plumtree FEC part.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq)]
#[tl(boxed, id = 0xc3e1c094)]
pub struct OverlayBroadcastPlumtreeFecToSign {
    pub id: Int256,
    pub timestamp: TlDouble,
    pub part_index: i32,
    pub tree_index: i32,
    pub data_size: i32,
    pub data_hash: Int256,
}

#[cfg(test)]
mod tests {
    use crate::{
        adnl::tl::AdnlTlPublicKey as TlPublicKey,
        overlay::tl::OverlayCertificate,
        tl::{TlInt256 as Int256, TlString},
    };

    use super::{
        OverlayBroadcastNotFound, OverlayBroadcastPlumtreeFec, OverlayBroadcastPlumtreeFecId,
        OverlayBroadcastPlumtreeFecToSign, OverlayBroadcastPlumtreeIHave,
        OverlayBroadcastPlumtreePrune, OverlayBroadcastPlumtreeSimple,
        OverlayBroadcastPlumtreeSimpleToSign, OverlayBroadcastPlumtreeUseful, OverlayCertificateId,
        OverlayCertificateIdV2, OverlayPlumtreeStatsDump, OverlayPlumtreeStatsExchange,
        OverlayPlumtreeStatsPush, OverlayPlumtreeStatsRecord, OverlayRepairPlumtreePart, TlDouble,
        TonNodeShardId,
    };
    use sha2::{Digest, Sha256};

    const SOURCE: TlPublicKey = TlPublicKey::Ed25519 {
        key: Int256([4; 32]),
    };

    fn record() -> OverlayPlumtreeStatsRecord {
        OverlayPlumtreeStatsRecord {
            src: Int256([5; 32]),
            epoch: 1,
            tree_index: 2,
            eager_0: vec![3, 4],
            eager_index: vec![5],
            delivered_bcsts: 6,
            parts_in_p99_bcsts: 7,
            useful_avg_ms: 8,
            useful_max_ms: 9,
            useful_p99_ms: 10,
        }
    }

    #[test]
    fn tl_double_preserves_ieee_bits() {
        for value in [0.0, -0.0, 1.0, 1_793_531_050.331_285_7, f64::INFINITY] {
            let bytes = tl_proto::serialize(TlDouble::from_f64(value));
            assert_eq!(bytes, value.to_bits().to_le_bytes());
            let decoded = tl_proto::deserialize::<TlDouble>(&bytes).unwrap();
            assert_eq!(decoded.to_bits(), value.to_bits());
            assert_eq!(decoded.to_f64().to_bits(), value.to_bits());
        }
    }

    #[test]
    fn golden_plumtree_signing_vectors_match_canonical_layout() {
        let simple = OverlayBroadcastPlumtreeSimpleToSign {
            id: Int256([0x11; 32]),
            timestamp: TlDouble::from_f64(1000.5),
            tree_index: 0,
            data_size: 6,
            data_hash: Int256(
                hex::decode("dd56de4137951d9c92681b03416ec15f886b4482a27e3a517d32f085244cbe5d")
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
        };
        assert_eq!(
            hex::encode(tl_proto::serialize(&simple)),
            "8e33f33811111111111111111111111111111111111111111111111111111111111111110000000000448f400000000006000000dd56de4137951d9c92681b03416ec15f886b4482a27e3a517d32f085244cbe5d"
        );

        let fec_id = OverlayBroadcastPlumtreeFecId {
            flags: 3,
            full_data_hash: Int256([0x22; 32]),
            full_data_size: 1234,
            part_size: 77,
        };
        let preimage = tl_proto::serialize(&fec_id);
        assert_eq!(
            hex::encode(&preimage),
            "3450a206030000002222222222222222222222222222222222222222222222222222222222222222d20400004d000000"
        );
        assert_eq!(
            hex::encode(Sha256::digest(&preimage)),
            "01bcf8ce4184a5c232e803c1003f439aec6e6b5fbd08dbf9c70d3ae66e1fbf83"
        );

        let fec = OverlayBroadcastPlumtreeFecToSign {
            id: Int256([0x33; 32]),
            timestamp: TlDouble::from_f64(2000.25),
            part_index: 2,
            tree_index: 3,
            data_size: 64,
            data_hash: Int256([0x44; 32]),
        };
        assert_eq!(
            hex::encode(tl_proto::serialize(&fec)),
            "94c0e1c333333333333333333333333333333333333333333333333333333333333333330000000000419f400200000003000000400000004444444444444444444444444444444444444444444444444444444444444444"
        );
    }

    #[test]
    fn plumtree_broadcast_constructors_roundtrip() {
        let timestamp = TlDouble::from_f64(1_793_531_050.5);
        let simple = OverlayBroadcastPlumtreeSimple {
            flags: 6,
            timestamp,
            src: SOURCE,
            certificate: OverlayCertificate::Empty,
            broadcast_id: Int256([1; 32]),
            tree_index: 0,
            data: vec![1, 2, 3],
            signature: vec![7; 64],
        };
        let bytes = tl_proto::serialize(&simple);
        assert_eq!(&bytes[..4], &0x3eccd26du32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayBroadcastPlumtreeSimple>(&bytes).unwrap(),
            simple
        );

        let fec = OverlayBroadcastPlumtreeFec {
            flags: 6,
            timestamp,
            src: SOURCE,
            certificate: OverlayCertificate::Empty,
            full_data_hash: Int256([2; 32]),
            full_data_size: 4096,
            part_index: 3,
            tree_index: 4,
            data: vec![9; 128],
            signature: vec![8; 64],
        };
        let bytes = tl_proto::serialize(&fec);
        assert_eq!(&bytes[..4], &0x8cba5fcfu32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayBroadcastPlumtreeFec>(&bytes).unwrap(),
            fec
        );

        let ihave = OverlayBroadcastPlumtreeIHave {
            broadcast_id: Int256([3; 32]),
            timestamp,
            part_index: 5,
            tree_index: 6,
            src: SOURCE,
            certificate: OverlayCertificate::Empty,
            payload_timestamp: timestamp,
            data_size: 1024,
            data_hash: Int256([4; 32]),
            signature: vec![9; 64],
        };
        let bytes = tl_proto::serialize(&ihave);
        assert_eq!(&bytes[..4], &0x6150c07bu32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayBroadcastPlumtreeIHave>(&bytes).unwrap(),
            ihave
        );

        let prune = OverlayBroadcastPlumtreePrune {
            broadcast_id: Int256([5; 32]),
            timestamp,
            part_index: 1,
            tree_index: 2,
        };
        let bytes = tl_proto::serialize(&prune);
        assert_eq!(&bytes[..4], &0xf0f73196u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayBroadcastPlumtreePrune>(&bytes).unwrap(),
            prune
        );

        let useful = OverlayBroadcastPlumtreeUseful {
            broadcast_id: Int256([6; 32]),
            timestamp,
            part_index: 3,
            tree_index: 4,
        };
        let bytes = tl_proto::serialize(&useful);
        assert_eq!(&bytes[..4], &0xaf825e83u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayBroadcastPlumtreeUseful>(&bytes).unwrap(),
            useful
        );

        let repair = OverlayRepairPlumtreePart {
            broadcast_id: Int256([7; 32]),
            timestamp,
            part_index: 8,
            tree_index: 9,
        };
        let bytes = tl_proto::serialize(&repair);
        assert_eq!(&bytes[..4], &0x7bd0590du32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayRepairPlumtreePart>(&bytes).unwrap(),
            repair
        );

        let not_found = OverlayBroadcastNotFound;
        let bytes = tl_proto::serialize(&not_found);
        assert_eq!(bytes, 0x95863624u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayBroadcastNotFound>(&bytes).unwrap(),
            not_found
        );
    }

    #[test]
    fn plumtree_statistics_constructors_roundtrip() {
        let record = record();

        let push = OverlayPlumtreeStatsPush {
            record: record.clone(),
        };
        let bytes = tl_proto::serialize(&push);
        assert_eq!(&bytes[..4], &0x22dfaa6eu32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayPlumtreeStatsPush>(&bytes).unwrap(),
            push
        );

        let shard = TonNodeShardId {
            workchain: -1,
            shard: i64::MIN,
        };
        let exchange = OverlayPlumtreeStatsExchange {
            overlay: Int256([1; 32]),
            overlay_type: TlString::new("validator".to_owned()),
            shard: shard.clone(),
            records: vec![record.clone()],
        };
        let bytes = tl_proto::serialize(&exchange);
        assert_eq!(&bytes[..4], &0x741db2e9u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayPlumtreeStatsExchange>(&bytes).unwrap(),
            exchange
        );

        let dump = OverlayPlumtreeStatsDump {
            overlay: Int256([2; 32]),
            overlay_type: TlString::new("catchain".to_owned()),
            shard,
            received_from: Int256([3; 32]),
            received_at: TlDouble::from_f64(1_793_531_050.0),
            records: vec![record],
        };
        let bytes = tl_proto::serialize(&dump);
        assert_eq!(&bytes[..4], &0xd7442202u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayPlumtreeStatsDump>(&bytes).unwrap(),
            dump
        );
    }

    #[test]
    fn certificate_id_constructors_roundtrip() {
        let v1 = OverlayCertificateId {
            overlay_id: Int256([1; 32]),
            node: Int256([2; 32]),
            expire_at: 3,
            max_size: 4,
        };
        let bytes = tl_proto::serialize(&v1);
        assert_eq!(&bytes[..4], &0x8fae60b9u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayCertificateId>(&bytes).unwrap(),
            v1
        );

        let v2 = OverlayCertificateIdV2 {
            overlay_id: Int256([1; 32]),
            node: Int256([2; 32]),
            expire_at: 3,
            max_size: 4,
            flags: 5,
        };
        let bytes = tl_proto::serialize(&v2);
        assert_eq!(&bytes[..4], &0xfc6cd2a7u32.to_le_bytes());
        assert_eq!(
            tl_proto::deserialize::<OverlayCertificateIdV2>(&bytes).unwrap(),
            v2
        );
    }
}
