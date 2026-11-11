//! Provider-neutral data types.

use std::sync::Arc;

use num_bigint::BigUint;

use crate::{
    liteclient::tl::LiteBlockIdExt,
    tl::TlInt256 as Int256,
    tlb::AccountStatus,
    tvm::{Cell, TvmStack},
};

/// A full block identifier independent of any particular provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockId {
    pub workchain: i32,
    pub shard: i64,
    pub seqno: i32,
    pub root_hash: Int256,
    pub file_hash: Int256,
}

impl From<LiteBlockIdExt> for BlockId {
    fn from(value: LiteBlockIdExt) -> Self {
        Self {
            workchain: value.workchain,
            shard: value.shard,
            seqno: value.seqno,
            root_hash: value.root_hash,
            file_hash: value.file_hash,
        }
    }
}

impl From<&LiteBlockIdExt> for BlockId {
    fn from(value: &LiteBlockIdExt) -> Self {
        Self {
            workchain: value.workchain,
            shard: value.shard,
            seqno: value.seqno,
            root_hash: value.root_hash.clone(),
            file_hash: value.file_hash.clone(),
        }
    }
}

impl From<BlockId> for LiteBlockIdExt {
    fn from(value: BlockId) -> Self {
        Self {
            workchain: value.workchain,
            shard: value.shard,
            seqno: value.seqno,
            root_hash: value.root_hash,
            file_hash: value.file_hash,
        }
    }
}

/// Latest masterchain information exposed by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterchainInfo {
    pub last: BlockId,
}

/// A high-level account state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountState {
    pub status: AccountStatus,
    pub balance: BigUint,
    pub code: Option<Arc<Cell>>,
    pub data: Option<Arc<Cell>>,
    pub last_transaction_lt: Option<u64>,
    pub last_transaction_hash: Option<Int256>,
    pub frozen_hash: Option<Int256>,
}

impl AccountState {
    pub fn empty(status: AccountStatus) -> Self {
        Self {
            status,
            balance: BigUint::from(0u8),
            code: None,
            data: None,
            last_transaction_lt: None,
            last_transaction_hash: None,
            frozen_hash: None,
        }
    }
}

/// A decoded get-method result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunMethodResult {
    pub exit_code: i32,
    pub stack: TvmStack,
    pub block: Option<BlockId>,
}

/// Result of submitting an external message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessageResult {
    /// LiteAPI `sendMessage` status code when the provider returns one.
    pub status_code: Option<u32>,
    /// Message hash when the provider returns it.
    pub message_hash: Option<Int256>,
}

/// Provider-neutral transaction summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub lt: u64,
    pub hash: Int256,
    pub utime: Option<u64>,
    pub raw_boc: Option<Vec<u8>>,
}

/// Provider-neutral transaction list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransactionList {
    pub transactions: Vec<Transaction>,
}

/// How a get-method is identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodRef<'a> {
    /// Human-readable method name, hashed with the TON CRC16 convention.
    Name(&'a str),
    /// Numeric method id.
    Id(u64),
}

impl MethodRef<'_> {
    pub fn resolve_id(self) -> u64 {
        match self {
            Self::Name(name) => crate::tvm::method_name_to_id(name),
            Self::Id(id) => id,
        }
    }
}
