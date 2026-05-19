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
    Account, Block, ConfigParams, MerkleProof, MerkleUpdate, MsgAddressInt, ShardAccount,
    ShardState, TlbDeserialize, Transaction,
};
use crate::tvm::{
    Address, BocInspection, Cell, deserialize_boc, deserialize_boc_roots, inspect_boc,
};
use anyhow::{Context, Result, bail};
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

/// Verified shard-account extraction from account-state proof material.
#[derive(Debug, Clone)]
pub struct ExtractedShardAccount {
    /// Raw proof/root BoC bytes used for extraction.
    pub proof: DecodedBoc,
    /// Shard account decoded from the proof-anchored root.
    pub shard_account: ShardAccount,
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

/// Extracts a shard account from proof material and validates it against the
/// requested address and account-state cell.
///
/// This helper intentionally does not treat generic BoC inspection as trust.
/// The current deterministic path accepts a BoC whose root is the
/// proof-anchored `ShardAccount` cell already obtained from a verified
/// dictionary path. Broader Merkle proof and shard-state dictionary traversal
/// remains a caller-visible follow-up; malformed roots, wrong account hashes,
/// and state/proof mismatches are rejected here.
pub fn extract_verified_shard_account(
    proof_raw: impl AsRef<[u8]>,
    state_raw: impl AsRef<[u8]>,
    account: &Address,
) -> Result<ExtractedShardAccount> {
    extract_verified_shard_account_with_root_hash(proof_raw, state_raw, account, None)
}

/// Extracts a shard account while also checking the proof/root hash expected
/// by an independently verified shard root.
pub fn extract_verified_shard_account_with_root_hash(
    proof_raw: impl AsRef<[u8]>,
    state_raw: impl AsRef<[u8]>,
    account: &Address,
    expected_root_hash: Option<[u8; 32]>,
) -> Result<ExtractedShardAccount> {
    let proof = DecodedBoc::decode(proof_raw)?;
    if let Some(expected) = expected_root_hash {
        let actual = proof.root.hash();
        if actual != expected {
            bail!("wrong shard root for shard account proof");
        }
    }
    let shard_account = ShardAccount::from_cell(proof.root.clone())
        .context("failed to decode verified ShardAccount root")?;
    validate_shard_account(&shard_account, state_raw.as_ref(), account)?;
    Ok(ExtractedShardAccount {
        proof,
        shard_account,
    })
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

    /// Builds a decoded account-state view and extracts a proof-anchored
    /// `ShardAccount` when the account proof contains a checked shard-account
    /// root for `account`.
    pub fn from_raw_verified(raw: AccountState, account: &Address) -> Result<Self> {
        let mut decoded = Self::from_raw(raw)?;
        if !decoded.raw.proof.is_empty() && !decoded.raw.state.is_empty() {
            decoded.shard_account = Some(
                extract_verified_shard_account(&decoded.raw.proof, &decoded.raw.state, account)?
                    .shard_account,
            );
        }
        Ok(decoded)
    }

    /// Converts the decoded state into a compact friendly account view.
    pub fn simple(&self) -> SimpleAccount {
        let shard_account = self.shard_account.as_ref();
        let last_transaction_lt =
            shard_account
                .map(|account| account.last_trans_lt)
                .or_else(|| {
                    self.account.as_ref().and_then(|account| match account {
                        Account::None => None,
                        Account::Full { storage, .. } => Some(storage.last_trans_lt),
                    })
                });
        let last_transaction_hash = shard_account.map(|account| account.last_trans_hash);
        SimpleAccount {
            block_id: self.raw.id.clone(),
            shard_block_id: self.raw.shardblk.clone(),
            last_transaction_lt,
            last_transaction_hash,
            state: self
                .account
                .as_ref()
                .map(simple_account_state)
                .unwrap_or(SimpleAccountState::None),
            account: self.account.clone(),
        }
    }
}

fn validate_shard_account(
    shard_account: &ShardAccount,
    state_raw: &[u8],
    expected_account: &Address,
) -> Result<()> {
    match &shard_account.account {
        Account::None => {}
        Account::Full { addr, .. } => validate_account_address(addr, expected_account)?,
    }

    if !state_raw.is_empty() {
        let state = decode_account_state_boc(state_raw)
            .context("failed to decode liteServer.accountState.state for proof validation")?
            .account;
        if state != shard_account.account {
            bail!("account state/proof mismatch");
        }
    }

    Ok(())
}

fn validate_account_address(actual: &MsgAddressInt, expected: &Address) -> Result<()> {
    match actual {
        MsgAddressInt::Std { address, .. } if address == expected => Ok(()),
        MsgAddressInt::Std { .. } => bail!("wrong account hash in shard account"),
        MsgAddressInt::Var { .. } => bail!("shard account uses non-standard account address"),
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

    #[test]
    fn verified_shard_account_extraction_populates_last_transaction() {
        let address = Address::new(0, [0x11; 32]);
        let account = full_account_for(address.clone(), 42, 123_456);
        let shard_account = shard_account(account.clone(), [0x55; 32], 42);
        let raw = AccountState {
            proof: serialize_boc(&shard_account.to_cell().unwrap(), false).unwrap(),
            state: serialize_boc(&account.to_cell().unwrap(), false).unwrap(),
            ..account_state_response(Vec::new())
        };

        let decoded = DecodedAccountState::from_raw_verified(raw, &address).unwrap();
        let simple = decoded.simple();

        assert_eq!(decoded.shard_account, Some(shard_account));
        assert_eq!(simple.last_transaction_lt, Some(42));
        assert_eq!(simple.last_transaction_hash, Some([0x55; 32]));
    }

    #[test]
    fn verified_shard_account_rejects_wrong_account_hash() {
        let requested = Address::new(0, [0x11; 32]);
        let actual = Address::new(0, [0x22; 32]);
        let account = full_account_for(actual, 42, 123_456);
        let proof = serialize_boc(
            &shard_account(account.clone(), [0x55; 32], 42)
                .to_cell()
                .unwrap(),
            false,
        )
        .unwrap();
        let state = serialize_boc(&account.to_cell().unwrap(), false).unwrap();

        let error = extract_verified_shard_account(proof, state, &requested)
            .unwrap_err()
            .to_string();
        assert!(error.contains("wrong account hash"));
    }

    #[test]
    fn verified_shard_account_rejects_wrong_shard_root() {
        let address = Address::new(0, [0x11; 32]);
        let account = full_account_for(address.clone(), 42, 123_456);
        let proof = serialize_boc(
            &shard_account(account.clone(), [0x55; 32], 42)
                .to_cell()
                .unwrap(),
            false,
        )
        .unwrap();
        let state = serialize_boc(&account.to_cell().unwrap(), false).unwrap();

        let error =
            extract_verified_shard_account_with_root_hash(proof, state, &address, Some([0x99; 32]))
                .unwrap_err()
                .to_string();
        assert!(error.contains("wrong shard root"));
    }

    #[test]
    fn verified_shard_account_rejects_malformed_proof_boc() {
        let address = Address::new(0, [0x11; 32]);
        let account = full_account_for(address.clone(), 42, 123_456);
        let state = serialize_boc(&account.to_cell().unwrap(), false).unwrap();

        let error = extract_verified_shard_account([0xde, 0xad], state, &address)
            .unwrap_err()
            .to_string();
        assert!(error.contains("failed to decode BoC root cell"));
    }

    #[test]
    fn verified_shard_account_rejects_state_proof_mismatch() {
        let address = Address::new(0, [0x11; 32]);
        let account = full_account_for(address.clone(), 42, 123_456);
        let mismatched_state = full_account_for(address.clone(), 43, 123_456);
        let proof = serialize_boc(
            &shard_account(account, [0x55; 32], 42).to_cell().unwrap(),
            false,
        )
        .unwrap();
        let state = serialize_boc(&mismatched_state.to_cell().unwrap(), false).unwrap();

        let error = extract_verified_shard_account(proof, state, &address)
            .unwrap_err()
            .to_string();
        assert!(error.contains("account state/proof mismatch"));
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
        full_account_for(Address::new(0, [0x11; 32]), last_trans_lt, grams)
    }

    fn full_account_for(address: Address, last_trans_lt: u64, grams: u64) -> crate::tlb::Account {
        crate::tlb::Account::Full {
            addr: MsgAddressInt::std(address),
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

    fn shard_account(
        account: crate::tlb::Account,
        last_trans_hash: [u8; 32],
        last_trans_lt: u64,
    ) -> ShardAccount {
        ShardAccount {
            account,
            last_trans_hash,
            last_trans_lt,
        }
    }
}
