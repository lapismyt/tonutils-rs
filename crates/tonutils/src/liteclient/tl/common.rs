use core::fmt;

use crate::tl::{TlInt256 as Int256, utils::fmt_bytes};
use derivative::Derivative;
use tl_proto::{TlRead, TlWrite};

/// tonNode.blockId workchain:int shard:long seqno:int = tonNode.BlockId;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteBlockId {
    pub workchain: i32,
    pub shard: i64,
    pub seqno: i32,
}

/// tonNode.blockIdExt workchain:int shard:long seqno:int root_hash:int256 file_hash:int256 = tonNode.BlockIdExt;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiteBlockIdExt {
    pub workchain: i32,
    pub shard: i64,
    pub seqno: i32,
    pub root_hash: Int256,
    pub file_hash: Int256,
}

impl fmt::Display for LiteBlockIdExt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({},{:X},{}):{}:{}",
            self.workchain, self.shard, self.seqno, self.root_hash, self.file_hash
        )
    }
}

/// liteServer.accountId workchain:int id:int256 = liteServer.AccountId;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteAccountId {
    pub workchain: i32,
    pub id: Int256,
}

/// liteServer.transactionId3 account:int256 lt:long = liteServer.TransactionId3;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteTransactionId3 {
    pub account: Int256,
    pub lt: u64,
}

/// liteServer.signature node_id_short:int256 signature:bytes = liteServer.Signature;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteSignature {
    pub node_id_short: Int256,
    #[derivative(Debug(format_with = "fmt_bytes"))]
    pub signature: Vec<u8>,
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
#[tl(
    boxed,
    scheme_inline = r##"liteServer.signatureSet.ordinary#f644a6e6 validator_set_hash:int catchain_seqno:int signatures:(vector liteServer.signature) = liteServer.SignatureSet;
        liteServer.signatureSet.simplex cc_seqno:int validator_set_hash:int signatures:(vector liteServer.signature) session_id:int256 slot:int candidate:bytes = liteServer.SignatureSet;"##
)]
pub enum LiteSignatureSet {
    /// liteServer.signatureSet.ordinary#f644a6e6 validator_set_hash:int catchain_seqno:int signatures:(vector liteServer.signature) = liteServer.SignatureSet;
    #[tl(id = 0xf644a6e6)]
    Ordinary {
        validator_set_hash: i32,
        catchain_seqno: i32,
        signatures: Vec<LiteSignature>,
    },
    /// liteServer.signatureSet.simplex cc_seqno:int validator_set_hash:int signatures:(vector liteServer.signature) session_id:int256 slot:int candidate:bytes = liteServer.SignatureSet;
    #[tl(id = "liteServer.signatureSet.simplex")]
    Simplex {
        cc_seqno: i32,
        validator_set_hash: i32,
        signatures: Vec<LiteSignature>,
        session_id: Int256,
        slot: i32,
        #[derivative(Debug(format_with = "fmt_bytes"))]
        candidate: Vec<u8>,
    },
}

#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
#[tl(
    boxed,
    scheme_inline = r##"liteServer.blockLinkBack to_key_block:Bool from:tonNode.blockIdExt to:tonNode.blockIdExt dest_proof:bytes proof:bytes state_proof:bytes = liteServer.BlockLink;
        liteServer.blockLinkForward to_key_block:Bool from:tonNode.blockIdExt to:tonNode.blockIdExt dest_proof:bytes config_proof:bytes signatures:liteServer.SignatureSet = liteServer.BlockLink;"##
)]
pub enum LiteBlockLink {
    /// liteServer.blockLinkBack to_key_block:Bool from:tonNode.blockIdExt to:tonNode.blockIdExt dest_proof:bytes proof:bytes state_proof:bytes = liteServer.BlockLink;
    #[tl(id = "liteServer.blockLinkBack")]
    BlockLinkBack {
        to_key_block: bool,
        from: LiteBlockIdExt,
        to: LiteBlockIdExt,
        #[derivative(Debug(format_with = "fmt_bytes"))]
        dest_proof: Vec<u8>,
        #[derivative(Debug(format_with = "fmt_bytes"))]
        proof: Vec<u8>,
        #[derivative(Debug(format_with = "fmt_bytes"))]
        state_proof: Vec<u8>,
    },
    /// liteServer.blockLinkForward to_key_block:Bool from:tonNode.blockIdExt to:tonNode.blockIdExt dest_proof:bytes config_proof:bytes signatures:liteServer.SignatureSet = liteServer.BlockLink;
    #[tl(id = "liteServer.blockLinkForward")]
    BlockLinkForward {
        to_key_block: bool,
        from: LiteBlockIdExt,
        to: LiteBlockIdExt,
        #[derivative(Debug(format_with = "fmt_bytes"))]
        dest_proof: Vec<u8>,
        #[derivative(Debug(format_with = "fmt_bytes"))]
        config_proof: Vec<u8>,
        signatures: LiteSignatureSet,
    },
}

/// tonNode.zeroStateIdExt workchain:int root_hash:int256 file_hash:int256 = tonNode.ZeroStateIdExt;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteZeroStateIdExt {
    pub workchain: i32,
    #[tl(size_hint = 32)]
    pub root_hash: Int256,
    #[tl(size_hint = 32)]
    pub file_hash: Int256,
}

/// liteServer.transactionMetadata depth:int initiator:liteServer.accountId initiator_lt:long = liteServer.TransactionMetadata;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteTransactionMetadata {
    #[tl(flags)]
    mode: (),
    depth: u32,
    initiator: LiteAccountId,
    initiator_lt: u64,
}

/// liteServer.transactionId mode:# account:mode.0?int256 lt:mode.1?long hash:mode.2?int256 metadata:mode.8?liteServer.transactionMetadata = liteServer.TransactionId;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteTransactionId {
    #[tl(flags)]
    pub mode: (),
    #[tl(flags_bit = "mode.0")]
    pub account: Option<Int256>,
    #[tl(flags_bit = "mode.1")]
    pub lt: Option<u64>,
    #[tl(flags_bit = "mode.2")]
    pub hash: Option<Int256>,
    #[tl(flags_bit = "mode.8")]
    pub metadata: Option<LiteTransactionMetadata>,
}

/// liteServer.libraryEntry hash:int256 data:bytes = liteServer.LibraryEntry;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteLibraryEntry {
    pub hash: Int256,
    #[derivative(Debug(format_with = "fmt_bytes"))]
    pub data: Vec<u8>,
}

/// liteServer.nonfinal.candidateId block_id:tonNode.blockIdExt creator:int256 collated_data_hash:int256 = liteServer.nonfinal.CandidateId;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteNonfinalCandidateId {
    pub block_id: LiteBlockIdExt,
    pub creator: Int256,
    pub collated_data_hash: Int256,
}

/// liteServer.nonfinal.candidateInfo id:liteServer.nonfinal.candidateId available:Bool approved_weight:long signed_weight:long total_weight:long = liteServer.nonfinal.CandidateInfo;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct LiteNonfinalCandidateInfo {
    pub id: LiteNonfinalCandidateId,
    pub available: bool,
    pub approved_weight: u64,
    pub signed_weight: u64,
    pub total_weight: u64,
}
