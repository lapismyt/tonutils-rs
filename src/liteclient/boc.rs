//! LiteClient BoC decode helpers.
//!
//! Helpers in this module preserve the original bytes and expose decoded cells
//! or TL-B views. Proof verification is explicit and opt-in.

use crate::tl::{
    common::{BlockIdExt, Int256},
    response::{
        AccountState, AllShardsInfo, BlockData, BlockHeader, BlockTransactionsExt, ConfigInfo,
        LibraryResultWithProof, ShardInfo, TransactionInfo,
    },
};
use crate::tlb::{
    Account, Block, ConfigParams, MerkleProof, MerkleUpdate, ShardAccount, ShardState,
    TlbDeserialize, Transaction,
};
use crate::tvm::{BocInspection, Cell, deserialize_boc, deserialize_boc_roots, inspect_boc};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;

/// Raw BoC bytes with a decoded root cell.
#[derive(Debug, Clone)]
pub struct DecodedBoc {
    /// Original BoC bytes.
    pub raw: Vec<u8>,
    /// Decoded root cell.
    pub root: Arc<Cell>,
}

impl DecodedBoc {
    /// Decodes a BoC and preserves the original bytes.
    pub fn decode(raw: impl AsRef<[u8]>) -> Result<Self> {
        let raw = raw.as_ref().to_vec();
        let root = deserialize_boc(&raw).context("failed to decode BoC root cell")?;
        Ok(Self { raw, root })
    }

    /// Root representation hash as lowercase hex.
    pub fn root_hash_hex(&self) -> String {
        hex::encode(self.root.hash())
    }
}

/// Raw BoC bytes with all decoded root cells.
#[derive(Debug, Clone)]
pub struct DecodedBocRoots {
    /// Original BoC bytes.
    pub raw: Vec<u8>,
    /// Decoded root cells in BoC root-index order.
    pub roots: Vec<Arc<Cell>>,
}

impl DecodedBocRoots {
    /// Decodes all BoC roots and preserves the original bytes.
    pub fn decode(raw: impl AsRef<[u8]>) -> Result<Self> {
        let raw = raw.as_ref().to_vec();
        let roots = deserialize_boc_roots(&raw).context("failed to decode BoC root cells")?;
        Ok(Self { raw, roots })
    }

    /// Root representation hashes as lowercase hex strings.
    pub fn root_hashes_hex(&self) -> Vec<String> {
        self.roots
            .iter()
            .map(|root| hex::encode(root.hash()))
            .collect()
    }
}

/// Raw proof BoC bytes with structural root metadata.
#[derive(Debug, Clone)]
pub struct InspectedProofBoc {
    /// Original BoC bytes.
    pub raw: Vec<u8>,
    /// Structural BoC inspection result.
    pub inspection: BocInspection,
}

impl InspectedProofBoc {
    /// Inspects a proof BoC without constructing semantic cells.
    pub fn inspect(raw: impl AsRef<[u8]>) -> Result<Self> {
        let raw = raw.as_ref().to_vec();
        let inspection = inspect_boc(&raw).context("failed to inspect BoC roots")?;
        Ok(Self { raw, inspection })
    }

    /// Number of root cells declared by the BoC.
    pub fn root_count(&self) -> usize {
        self.inspection.root_count()
    }

    /// Root representation hashes as lowercase hex strings.
    pub fn root_hashes_hex(&self) -> Vec<String> {
        self.inspection.root_hashes_hex()
    }
}

/// Decoded account-state BoC.
#[derive(Debug, Clone)]
pub struct DecodedAccountStateBoc {
    /// Raw and root cell view.
    pub boc: DecodedBoc,
    /// Typed account, when the cell is an `Account`.
    pub account: Account,
}

/// Decoded block BoC.
#[derive(Debug, Clone)]
pub struct DecodedBlockBoc {
    /// Raw and root cell view.
    pub boc: DecodedBoc,
    /// Typed block wrapper.
    pub block: Block,
}

/// Decoded config proof BoC.
#[derive(Debug, Clone)]
pub struct DecodedConfigParamsBoc {
    /// Raw and root cell view.
    pub boc: DecodedBoc,
    /// Typed config parameters wrapper.
    pub config: ConfigParams,
}

/// Decoded shard-state or shard-proof BoC.
#[derive(Debug, Clone)]
pub struct DecodedShardStateBoc {
    /// Raw and root cell view.
    pub boc: DecodedBoc,
    /// Typed shard-state wrapper.
    pub shard_state: ShardState,
}

/// Opaque typed view for a shard descriptor cell.
#[derive(Debug, Clone)]
pub struct ShardDescr {
    /// Raw and root cell view.
    pub boc: DecodedBoc,
}

/// Decoded `liteServer.blockData`.
#[derive(Debug, Clone)]
pub struct DecodedBlockData {
    /// Original TL response.
    pub raw: BlockData,
    /// Decoded block BoC.
    pub data: DecodedBlockBoc,
}

/// Decoded `liteServer.blockHeader`.
#[derive(Debug, Clone)]
pub struct DecodedBlockHeader {
    /// Original TL response.
    pub raw: BlockHeader,
    /// Raw header proof BoC, decoded to a root cell.
    pub header_proof: DecodedBoc,
}

/// Decoded `liteServer.accountState`.
#[derive(Debug, Clone)]
pub struct DecodedAccountState {
    /// Original TL response.
    pub raw: AccountState,
    /// Structurally inspected shard proof roots when present.
    pub shard_proof: Option<InspectedProofBoc>,
    /// Structurally inspected account proof roots when present.
    pub proof: Option<InspectedProofBoc>,
    /// Decoded shard-account state, when extracted from proof dictionaries.
    pub shard_account: Option<ShardAccount>,
    /// Account decoded from `liteServer.accountState.state`.
    pub account: Option<Account>,
}

/// Pytoniq-like friendly account state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimpleAccountState {
    /// Account does not exist in the shard account.
    None,
    /// Account exists but is uninitialized.
    Uninit,
    /// Account is frozen.
    Frozen,
    /// Account is active.
    Active,
}

/// Pytoniq-like friendly account wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleAccount {
    /// Account block id used for the query.
    pub block_id: BlockIdExt,
    /// Shard block id returned by the liteserver.
    pub shard_block_id: BlockIdExt,
    /// Last transaction logical time, if the shard account was present.
    pub last_transaction_lt: Option<u64>,
    /// Last transaction hash, if the shard account was present.
    pub last_transaction_hash: Option<[u8; 32]>,
    /// Friendly account state.
    pub state: SimpleAccountState,
    /// Full decoded account model.
    pub account: Option<Account>,
}

/// Decoded single-transaction response.
#[derive(Debug, Clone)]
pub struct DecodedTransactionInfo {
    /// Original TL response.
    pub raw: TransactionInfo,
    /// Decoded proof BoC when present.
    pub proof: Option<DecodedBoc>,
    /// Decoded transaction when present.
    pub transaction: Option<Transaction>,
}

/// Decoded transaction-list response.
#[derive(Debug, Clone)]
pub struct DecodedTransactionList {
    /// Block ids returned by the liteserver.
    pub ids: Vec<BlockIdExt>,
    /// Raw transactions BoC bytes.
    pub raw_transactions: Vec<u8>,
    /// Transactions decoded from the BoC root when the payload contains a
    /// single transaction root.
    pub transactions: Vec<Transaction>,
}

/// Decoded block-transactions-ext response.
#[derive(Debug, Clone)]
pub struct DecodedBlockTransactionsExt {
    /// Original TL response.
    pub raw: BlockTransactionsExt,
    /// Transactions decoded from the BoC root when the payload contains a
    /// single transaction root.
    pub transactions: Vec<Transaction>,
    /// Decoded proof BoC when present.
    pub proof: Option<DecodedBoc>,
}

/// Decoded shard-info response.
#[derive(Debug, Clone)]
pub struct DecodedShardInfo {
    /// Original TL response.
    pub raw: ShardInfo,
    /// Decoded shard proof when present.
    pub shard_proof: Option<DecodedBoc>,
    /// Opaque shard descriptor cell.
    pub shard_descr: ShardDescr,
}

/// Decoded all-shards-info response.
#[derive(Debug, Clone)]
pub struct DecodedAllShardsInfo {
    /// Original TL response.
    pub raw: AllShardsInfo,
    /// Decoded proof when present.
    pub proof: Option<DecodedBoc>,
    /// Opaque root for the shard dictionary payload.
    pub data: DecodedBoc,
}

/// Decoded config response.
#[derive(Debug, Clone)]
pub struct DecodedConfigInfo {
    /// Original TL response.
    pub raw: ConfigInfo,
    /// Decoded state proof when present.
    pub state_proof: Option<DecodedBoc>,
    /// Decoded config proof when present.
    pub config_proof: Option<DecodedConfigParamsBoc>,
}

/// Decoded libraries-with-proof response.
#[derive(Debug, Clone)]
pub struct DecodedLibrariesWithProof {
    /// Original TL response.
    pub raw: LibraryResultWithProof,
    /// Library cells by requested hash. Empty library bytes map to `None`.
    pub libraries: HashMap<Int256, Option<Arc<Cell>>>,
    /// Decoded state proof when present.
    pub state_proof: Option<DecodedBoc>,
    /// Decoded data proof when present.
    pub data_proof: Option<DecodedBoc>,
}

/// Decodes a raw account-state BoC into `Account`.
pub fn decode_account_state_boc(raw: impl AsRef<[u8]>) -> Result<DecodedAccountStateBoc> {
    let boc = DecodedBoc::decode(raw)?;
    let account = Account::from_cell(boc.root.clone()).context("failed to decode Account TL-B")?;
    Ok(DecodedAccountStateBoc { boc, account })
}

/// Decodes a raw block BoC into the Phase 1 `Block` wrapper.
pub fn decode_block_boc(raw: impl AsRef<[u8]>) -> Result<DecodedBlockBoc> {
    let boc = DecodedBoc::decode(raw)?;
    let block = Block::from_cell(boc.root.clone()).context("failed to decode Block TL-B")?;
    Ok(DecodedBlockBoc { boc, block })
}

/// Decodes a raw config-params proof payload into `ConfigParams`.
pub fn decode_config_params_boc(raw: impl AsRef<[u8]>) -> Result<DecodedConfigParamsBoc> {
    let boc = DecodedBoc::decode(raw)?;
    let config =
        ConfigParams::from_cell(boc.root.clone()).context("failed to decode ConfigParams TL-B")?;
    Ok(DecodedConfigParamsBoc { boc, config })
}

/// Decodes a raw shard-state BoC.
pub fn decode_shard_state_boc(raw: impl AsRef<[u8]>) -> Result<DecodedShardStateBoc> {
    let boc = DecodedBoc::decode(raw)?;
    let shard_state =
        ShardState::from_cell(boc.root.clone()).context("failed to decode ShardState TL-B")?;
    Ok(DecodedShardStateBoc { boc, shard_state })
}

/// Decodes a raw shard-account BoC.
pub fn decode_shard_account_boc(raw: impl AsRef<[u8]>) -> Result<ShardAccount> {
    let boc = DecodedBoc::decode(raw)?;
    ShardAccount::from_cell(boc.root).context("failed to decode ShardAccount TL-B")
}

/// Decodes a raw transaction BoC.
pub fn decode_transaction_boc(raw: impl AsRef<[u8]>) -> Result<Transaction> {
    let boc = DecodedBoc::decode(raw)?;
    Transaction::from_cell(boc.root).context("failed to decode Transaction TL-B")
}

pub(crate) fn decode_optional_boc(raw: &[u8]) -> Result<Option<DecodedBoc>> {
    if raw.is_empty() {
        Ok(None)
    } else {
        DecodedBoc::decode(raw).map(Some)
    }
}

pub(crate) fn inspect_optional_proof_boc(raw: &[u8]) -> Result<Option<InspectedProofBoc>> {
    if raw.is_empty() {
        Ok(None)
    } else {
        InspectedProofBoc::inspect(raw).map(Some)
    }
}

pub(crate) fn decode_optional_config(raw: &[u8]) -> Result<Option<DecodedConfigParamsBoc>> {
    if raw.is_empty() {
        Ok(None)
    } else {
        decode_config_params_boc(raw).map(Some)
    }
}

pub(crate) fn decode_single_transaction_list(raw: &[u8]) -> Result<Vec<Transaction>> {
    if raw.is_empty() {
        Ok(Vec::new())
    } else {
        decode_transaction_boc(raw).map(|transaction| vec![transaction])
    }
}

impl DecodedAccountState {
    /// Builds a decoded account-state view from the raw LiteAPI response.
    pub fn from_raw(raw: AccountState) -> Result<Self> {
        let shard_proof = inspect_optional_proof_boc(&raw.shard_proof)?;
        let proof = inspect_optional_proof_boc(&raw.proof)?;
        let account = if raw.state.is_empty() {
            None
        } else {
            Some(
                decode_account_state_boc(&raw.state)
                    .context("failed to decode liteServer.accountState.state")?
                    .account,
            )
        };
        Ok(Self {
            raw,
            shard_proof,
            proof,
            shard_account: None,
            account,
        })
    }

    /// Converts the decoded state into a compact friendly account view.
    pub fn simple(&self) -> SimpleAccount {
        let last_transaction_lt = self.account.as_ref().and_then(|account| match account {
            Account::None => None,
            Account::Full { storage, .. } => Some(storage.last_trans_lt),
        });
        SimpleAccount {
            block_id: self.raw.id.clone(),
            shard_block_id: self.raw.shardblk.clone(),
            last_transaction_lt,
            last_transaction_hash: None,
            state: self
                .account
                .as_ref()
                .map(simple_account_state)
                .unwrap_or(SimpleAccountState::None),
            account: self.account.clone(),
        }
    }
}

fn simple_account_state(account: &Account) -> SimpleAccountState {
    match account {
        Account::None => SimpleAccountState::None,
        Account::Full { storage, .. } => match &storage.state {
            crate::tlb::AccountState::Uninit => SimpleAccountState::Uninit,
            crate::tlb::AccountState::Frozen { .. } => SimpleAccountState::Frozen,
            crate::tlb::AccountState::Active { .. } => SimpleAccountState::Active,
        },
    }
}

/// Decodes an exotic Merkle proof cell. This does not verify trust roots.
pub fn decode_merkle_proof_boc(raw: impl AsRef<[u8]>) -> Result<MerkleProof> {
    let boc = DecodedBoc::decode(raw)?;
    MerkleProof::from_exotic_cell(boc.root).context("failed to decode MERKLE_PROOF exotic cell")
}

/// Decodes an exotic Merkle update cell. This does not verify trust roots.
pub fn decode_merkle_update_boc(raw: impl AsRef<[u8]>) -> Result<MerkleUpdate> {
    let boc = DecodedBoc::decode(raw)?;
    MerkleUpdate::from_exotic_cell(boc.root).context("failed to decode MERKLE_UPDATE exotic cell")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tl::common::Int256;
    use crate::tlb::{
        AccountState as TlbAccountState, AccountStatus, AccountStorage, CurrencyCollection, Grams,
        MsgAddressInt, StateInit, StorageExtraInfo, StorageInfo, StorageUsed, TlbSerialize,
    };
    use crate::tvm::{Address, Builder, serialize_boc};
    use num_bigint::BigUint;

    #[test]
    fn decoded_boc_preserves_raw_bytes_and_hash() {
        let cell = Builder::new().build().unwrap();
        let raw = serialize_boc(&cell, false).unwrap();
        let decoded = DecodedBoc::decode(&raw).unwrap();
        assert_eq!(decoded.raw, raw);
        assert_eq!(decoded.root_hash_hex(), hex::encode(cell.hash()));
    }

    #[test]
    fn account_state_helper_decodes_typed_account() {
        let account = crate::tlb::Account::None;
        let raw = serialize_boc(&account.to_cell().unwrap(), false).unwrap();
        let decoded = decode_account_state_boc(raw).unwrap();
        assert_eq!(decoded.account, crate::tlb::Account::None);
        let _ = AccountStatus::Active;
    }

    #[test]
    fn decoded_account_state_decodes_none_account_state_cell() {
        let account = crate::tlb::Account::None;
        let raw =
            account_state_response(serialize_boc(&account.to_cell().unwrap(), false).unwrap());

        let decoded = DecodedAccountState::from_raw(raw).unwrap();
        let simple = decoded.simple();

        assert_eq!(decoded.account, Some(crate::tlb::Account::None));
        assert_eq!(decoded.shard_account, None);
        assert_eq!(simple.state, SimpleAccountState::None);
        assert_eq!(simple.last_transaction_lt, None);
        assert_eq!(simple.last_transaction_hash, None);
        assert_eq!(decoded.shard_proof.as_ref().unwrap().root_count(), 2);
        assert_eq!(decoded.proof.as_ref().unwrap().root_count(), 2);
    }

    #[test]
    fn decoded_account_state_decodes_full_account_from_state_cell() {
        let account = full_account(42, 123_456);
        let raw =
            account_state_response(serialize_boc(&account.to_cell().unwrap(), false).unwrap());

        let decoded = DecodedAccountState::from_raw(raw).unwrap();
        let simple = decoded.simple();

        assert_eq!(decoded.account, Some(account));
        assert_eq!(simple.state, SimpleAccountState::Active);
        assert_eq!(simple.last_transaction_lt, Some(42));
        assert_eq!(simple.last_transaction_hash, None);
        assert_eq!(
            simple.account.as_ref().unwrap(),
            decoded.account.as_ref().unwrap()
        );
    }

    fn account_state_response(state: Vec<u8>) -> AccountState {
        AccountState {
            id: block_id(1),
            shardblk: block_id(2),
            shard_proof: two_root_boc(),
            proof: two_root_boc(),
            state,
        }
    }

    fn block_id(seqno: i32) -> BlockIdExt {
        BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno,
            root_hash: Int256([seqno as u8; 32]),
            file_hash: Int256([(seqno + 1) as u8; 32]),
        }
    }

    fn two_root_boc() -> Vec<u8> {
        hex::decode("b5ee9c72010102020005000100000002aa").unwrap()
    }

    fn full_account(last_trans_lt: u64, grams: u64) -> crate::tlb::Account {
        crate::tlb::Account::Full {
            addr: MsgAddressInt::std(Address::new(0, [0x11; 32])),
            storage_stat: StorageInfo {
                used: StorageUsed::new(BigUint::from(1u8), BigUint::from(64u8)),
                last_paid: 1_700_000_000,
                due_payment: None,
                extra: StorageExtraInfo::None,
            },
            storage: AccountStorage {
                last_trans_lt,
                balance: CurrencyCollection::grams(Grams::from(grams)),
                state: TlbAccountState::Active {
                    state_init: StateInit::empty(),
                },
            },
        }
    }
}
