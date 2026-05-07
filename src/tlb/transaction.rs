//! Hand-written TL-B codecs for account state, transactions, descriptions, and phases.

use crate::tlb::{
    AccStatusChange, CurrencyCollection, Grams, Message, MsgAddressInt, StateInit, StorageUsed,
    TrActionPhase,
};
use crate::tlb::{
    Result, TlbDeserialize, TlbError, TlbSerialize, ensure_empty, load_maybe, load_ref_tlb,
    load_var_uint, store_maybe, store_ref_tlb, store_tag, store_var_uint,
};
use crate::tvm::{Builder, HashmapAug, HashmapAugE, HashmapE, Slice};
use num_bigint::BigUint;

const OUT_MSG_KEY_BITS: usize = 15;
const VAR_UINT_3_LEN_BITS: usize = 2;
const VAR_UINT_3_MAX_BYTES: usize = 2;
const VAR_UINT_7_LEN_BITS: usize = 3;
const VAR_UINT_7_MAX_BYTES: usize = 6;

/// TL-B `StorageExtraInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageExtraInfo {
    /// `storage_extra_none$000`.
    None,
    /// `storage_extra_info$001 dict_hash:uint256`.
    Info {
        /// Hash of the account extra-currency dictionary.
        dict_hash: [u8; 32],
    },
}

impl TlbSerialize for StorageExtraInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::None => {
                store_tag(builder, "000")?;
            }
            Self::Info { dict_hash } => {
                store_tag(builder, "001")?;
                builder.store_bytes(dict_hash)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for StorageExtraInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let actual_bits = load_three_bit_tag(slice, "StorageExtraInfo", "000|001")?;
        match actual_bits.as_str() {
            "000" => Ok(Self::None),
            "001" => {
                let mut dict_hash = [0; 32];
                dict_hash.copy_from_slice(&slice.load_bytes(32)?);
                Ok(Self::Info { dict_hash })
            }
            _ => Err(TlbError::TagMismatch {
                constructor: "StorageExtraInfo",
                expected_bits: "000|001",
                actual_bits,
            }),
        }
    }
}

/// TL-B `storage_info$_ used:StorageUsed storage_extra:StorageExtraInfo last_paid:uint32 due_payment:(Maybe Grams) = StorageInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageInfo {
    /// Storage consumed by the account.
    pub used: StorageUsed,
    /// Unix time when storage fees were last paid.
    pub last_paid: u32,
    /// Storage fees due, if any.
    pub due_payment: Option<Grams>,
    /// Extra storage metadata.
    pub extra: StorageExtraInfo,
}

impl TlbSerialize for StorageInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.used.store_tlb(builder)?;
        self.extra.store_tlb(builder)?;
        builder.store_u32(self.last_paid)?;
        store_maybe(builder, &self.due_payment)?;
        Ok(())
    }
}

impl TlbDeserialize for StorageInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            used: StorageUsed::load_tlb(slice)?,
            extra: StorageExtraInfo::load_tlb(slice)?,
            last_paid: slice.load_u32()?,
            due_payment: load_maybe(slice)?,
        })
    }
}

/// TL-B `AccountState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountState {
    /// `account_uninit$00`.
    Uninit,
    /// `account_frozen$01 state_hash:bits256`.
    Frozen {
        /// Hash of the frozen state.
        state_hash: [u8; 32],
    },
    /// `account_active$1 _:StateInit`.
    Active {
        /// Active contract state.
        state_init: StateInit,
    },
}

impl TlbSerialize for AccountState {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Uninit => store_tag(builder, "00")?,
            Self::Frozen { state_hash } => {
                store_tag(builder, "01")?;
                builder.store_bytes(state_hash)?;
            }
            Self::Active { state_init } => {
                store_tag(builder, "1")?;
                state_init.store_tlb(builder)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for AccountState {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = load_tag_bit(slice, "AccountState", "00|01|1", "")?;
        if first {
            return Ok(Self::Active {
                state_init: StateInit::load_tlb(slice)?,
            });
        }
        let second = load_tag_bit(slice, "AccountState", "00|01|1", "0")?;
        if second {
            let mut state_hash = [0; 32];
            state_hash.copy_from_slice(&slice.load_bytes(32)?);
            Ok(Self::Frozen { state_hash })
        } else {
            Ok(Self::Uninit)
        }
    }
}

/// TL-B `account_storage$_ last_trans_lt:uint64 balance:CurrencyCollection state:AccountState = AccountStorage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountStorage {
    /// Last transaction logical time.
    pub last_trans_lt: u64,
    /// Current account balance.
    pub balance: CurrencyCollection,
    /// Current account state.
    pub state: AccountState,
}

impl TlbSerialize for AccountStorage {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u64(self.last_trans_lt)?;
        self.balance.store_tlb(builder)?;
        self.state.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for AccountStorage {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            last_trans_lt: slice.load_u64()?,
            balance: CurrencyCollection::load_tlb(slice)?,
            state: AccountState::load_tlb(slice)?,
        })
    }
}

/// TL-B `AccountStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountStatus {
    /// `account_status_uninit$00`.
    Uninit,
    /// `account_status_frozen$01`.
    Frozen,
    /// `account_status_active$10`.
    Active,
    /// `account_status_nonexist$11`.
    Nonexist,
}

impl TlbSerialize for AccountStatus {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Uninit => store_tag(builder, "00"),
            Self::Frozen => store_tag(builder, "01"),
            Self::Active => store_tag(builder, "10"),
            Self::Nonexist => store_tag(builder, "11"),
        }
    }
}

impl TlbDeserialize for AccountStatus {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_two_bit_tag(slice, "AccountStatus", "00|01|10|11")?.as_str() {
            "00" => Ok(Self::Uninit),
            "01" => Ok(Self::Frozen),
            "10" => Ok(Self::Active),
            "11" => Ok(Self::Nonexist),
            _ => unreachable!("two-bit AccountStatus tag is exhaustive"),
        }
    }
}

/// TL-B `Account`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Account {
    /// `account_none$0`.
    None,
    /// `account$1 addr:MsgAddressInt storage_stat:StorageInfo storage:AccountStorage`.
    Full {
        /// Account address.
        addr: MsgAddressInt,
        /// Storage accounting information.
        storage_stat: StorageInfo,
        /// Account storage payload.
        storage: AccountStorage,
    },
}

impl TlbSerialize for Account {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::None => {
                builder.store_bit(false)?;
            }
            Self::Full {
                addr,
                storage_stat,
                storage,
            } => {
                builder.store_bit(true)?;
                addr.store_tlb(builder)?;
                storage_stat.store_tlb(builder)?;
                storage.store_tlb(builder)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for Account {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        if slice.load_bit()? {
            Ok(Self::Full {
                addr: MsgAddressInt::load_tlb(slice)?,
                storage_stat: StorageInfo::load_tlb(slice)?,
                storage: AccountStorage::load_tlb(slice)?,
            })
        } else {
            Ok(Self::None)
        }
    }
}

/// TL-B `shard_account$_ account:^Account last_trans_hash:uint256 last_trans_lt:uint64 = ShardAccount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardAccount {
    /// Referenced account value.
    pub account: Account,
    /// Last transaction hash.
    pub last_trans_hash: [u8; 32],
    /// Last transaction logical time.
    pub last_trans_lt: u64,
}

impl TlbSerialize for ShardAccount {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_ref_tlb(builder, &self.account)?;
        builder.store_bytes(&self.last_trans_hash)?;
        builder.store_u64(self.last_trans_lt)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardAccount {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let account = load_ref_tlb(slice, "Account")?;
        let mut last_trans_hash = [0; 32];
        last_trans_hash.copy_from_slice(&slice.load_bytes(32)?);
        Ok(Self {
            account,
            last_trans_hash,
            last_trans_lt: slice.load_u64()?,
        })
    }
}

/// TL-B `depth_balance$_ split_depth:(#<= 30) balance:CurrencyCollection = DepthBalanceInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthBalanceInfo {
    /// Shard split depth, encoded in five bits and constrained to `0..=30`.
    pub split_depth: u8,
    /// Account or subtree balance augmentation.
    pub balance: CurrencyCollection,
}

impl TlbSerialize for DepthBalanceInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.split_depth > 30 {
            return Err(TlbError::CustomSchema {
                schema: "DepthBalanceInfo.split_depth",
                message: format!("value {} exceeds 30", self.split_depth),
            });
        }
        builder.store_uint(self.split_depth as u64, 5)?;
        self.balance.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for DepthBalanceInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let split_depth = slice.load_uint(5)? as u8;
        if split_depth > 30 {
            return Err(TlbError::NonCanonicalValue {
                schema: "DepthBalanceInfo.split_depth",
                reason: format!("value {split_depth} exceeds 30"),
            });
        }
        Ok(Self {
            split_depth,
            balance: CurrencyCollection::load_tlb(slice)?,
        })
    }
}

/// TL-B `_ (HashmapAugE 256 ShardAccount DepthBalanceInfo) = ShardAccounts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardAccounts {
    /// Augmented shard-account dictionary keyed by 256-bit account address hash.
    pub accounts: HashmapAugE<ShardAccount, DepthBalanceInfo>,
}

impl TlbSerialize for ShardAccounts {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.accounts.key_bits() != 256 {
            return Err(TlbError::CustomSchema {
                schema: "ShardAccounts",
                message: format!(
                    "account dictionary key width {} is not 256",
                    self.accounts.key_bits()
                ),
            });
        }
        builder
            .store_hashmap_aug_e_with(
                &self.accounts,
                |builder, account| account.store_tlb(builder).map_err(anyhow::Error::from),
                |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
            )
            .map_err(anyhow_to_tlb_error)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardAccounts {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            accounts: slice
                .load_hashmap_aug_e_with(
                    256,
                    |slice| ShardAccount::load_tlb(slice).map_err(anyhow::Error::from),
                    |slice| DepthBalanceInfo::load_tlb(slice).map_err(anyhow::Error::from),
                )
                .map_err(anyhow_to_tlb_error)?,
        })
    }
}

/// TL-B `acc_trans#5 ... = AccountBlock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBlock {
    /// Account address hash.
    pub account_addr: [u8; 32],
    /// Non-empty augmented transaction dictionary keyed by 64-bit logical time.
    pub transactions: HashmapAug<Transaction, CurrencyCollection>,
    /// Referenced account hash update.
    pub state_update: HashUpdateAccount,
}

impl TlbSerialize for AccountBlock {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.transactions.key_bits() != 64 {
            return Err(TlbError::CustomSchema {
                schema: "AccountBlock.transactions",
                message: format!(
                    "transaction dictionary key width {} is not 64",
                    self.transactions.key_bits()
                ),
            });
        }
        store_tag(builder, "0101")?;
        builder.store_bytes(&self.account_addr)?;
        builder
            .store_hashmap_aug_with(
                &self.transactions,
                |builder, transaction| {
                    store_ref_tlb(builder, transaction).map_err(anyhow::Error::from)
                },
                |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
            )
            .map_err(anyhow_to_tlb_error)?;
        store_ref_tlb(builder, &self.state_update)?;
        Ok(())
    }
}

impl TlbDeserialize for AccountBlock {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_tag_bits(slice, "AccountBlock", "0101")?;
        let mut account_addr = [0; 32];
        account_addr.copy_from_slice(&slice.load_bytes(32)?);
        Ok(Self {
            account_addr,
            transactions: slice
                .load_hashmap_aug_with(
                    64,
                    |slice| {
                        load_ref_tlb::<Transaction>(slice, "Transaction")
                            .map_err(anyhow::Error::from)
                    },
                    |slice| CurrencyCollection::load_tlb(slice).map_err(anyhow::Error::from),
                )
                .map_err(anyhow_to_tlb_error)?,
            state_update: load_ref_tlb(slice, "HASH_UPDATE Account")?,
        })
    }
}

/// TL-B `_ (HashmapAugE 256 AccountBlock CurrencyCollection) = ShardAccountBlocks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardAccountBlocks {
    /// Augmented account-block dictionary keyed by 256-bit account address hash.
    pub blocks: HashmapAugE<AccountBlock, CurrencyCollection>,
}

impl TlbSerialize for ShardAccountBlocks {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.blocks.key_bits() != 256 {
            return Err(TlbError::CustomSchema {
                schema: "ShardAccountBlocks",
                message: format!(
                    "account-block dictionary key width {} is not 256",
                    self.blocks.key_bits()
                ),
            });
        }
        builder
            .store_hashmap_aug_e_with(
                &self.blocks,
                |builder, block| block.store_tlb(builder).map_err(anyhow::Error::from),
                |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
            )
            .map_err(anyhow_to_tlb_error)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardAccountBlocks {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            blocks: slice
                .load_hashmap_aug_e_with(
                    256,
                    |slice| AccountBlock::load_tlb(slice).map_err(anyhow::Error::from),
                    |slice| CurrencyCollection::load_tlb(slice).map_err(anyhow::Error::from),
                )
                .map_err(anyhow_to_tlb_error)?,
        })
    }
}

/// Concrete TL-B `update_hashes#72 ... = HASH_UPDATE Account`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashUpdateAccount {
    /// Old account representation hash.
    pub old_hash: [u8; 32],
    /// New account representation hash.
    pub new_hash: [u8; 32],
}

impl TlbSerialize for HashUpdateAccount {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_uint(0x72, 8)?;
        builder.store_bytes(&self.old_hash)?;
        builder.store_bytes(&self.new_hash)?;
        Ok(())
    }
}

impl TlbDeserialize for HashUpdateAccount {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_u8_tag(slice, "HASH_UPDATE Account", "#72", 0x72)?;
        let mut old_hash = [0; 32];
        let mut new_hash = [0; 32];
        old_hash.copy_from_slice(&slice.load_bytes(32)?);
        new_hash.copy_from_slice(&slice.load_bytes(32)?);
        Ok(Self { old_hash, new_hash })
    }
}

/// TL-B `transaction$0111 ... = Transaction`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    /// Account address hash.
    pub account_addr: [u8; 32],
    /// Transaction logical time.
    pub lt: u64,
    /// Previous transaction hash.
    pub prev_trans_hash: [u8; 32],
    /// Previous transaction logical time.
    pub prev_trans_lt: u64,
    /// Unix time when the transaction was created.
    pub now: u32,
    /// Outbound message count, encoded as `uint15`.
    pub outmsg_cnt: u16,
    /// Original account status.
    pub orig_status: AccountStatus,
    /// Final account status.
    pub end_status: AccountStatus,
    /// Optional referenced inbound message from the child reference.
    pub in_msg: Option<Message>,
    /// Outbound messages from the child `HashmapE 15 ^(Message Any)`.
    pub out_msgs: HashmapE<Message>,
    /// Total transaction fees.
    pub total_fees: CurrencyCollection,
    /// Referenced account hash update.
    pub state_update: HashUpdateAccount,
    /// Referenced transaction description.
    pub description: TransactionDescr,
}

impl TlbSerialize for Transaction {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.outmsg_cnt > 0x7fff {
            return Err(TlbError::CustomSchema {
                schema: "Transaction.outmsg_cnt",
                message: format!("value {} does not fit in 15 bits", self.outmsg_cnt),
            });
        }
        if self.out_msgs.key_bits() != OUT_MSG_KEY_BITS {
            return Err(TlbError::CustomSchema {
                schema: "Transaction.out_msgs",
                message: format!(
                    "outbound message dictionary key width {} is not 15",
                    self.out_msgs.key_bits()
                ),
            });
        }

        store_tag(builder, "0111")?;
        builder.store_bytes(&self.account_addr)?;
        builder.store_u64(self.lt)?;
        builder.store_bytes(&self.prev_trans_hash)?;
        builder.store_u64(self.prev_trans_lt)?;
        builder.store_u32(self.now)?;
        builder.store_uint(self.outmsg_cnt as u64, OUT_MSG_KEY_BITS)?;
        self.orig_status.store_tlb(builder)?;
        self.end_status.store_tlb(builder)?;

        let mut messages = Builder::new();
        store_maybe_ref_message(&mut messages, &self.in_msg)?;
        messages.store_hashmap_e_with(&self.out_msgs, |builder, message| {
            store_ref_tlb(builder, message).map_err(anyhow::Error::from)
        })?;
        builder.store_ref(messages.build()?)?;

        self.total_fees.store_tlb(builder)?;
        store_ref_tlb(builder, &self.state_update)?;
        store_ref_tlb(builder, &self.description)?;
        Ok(())
    }
}

impl TlbDeserialize for Transaction {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_tag_bits(slice, "Transaction", "0111")?;
        let mut account_addr = [0; 32];
        let mut prev_trans_hash = [0; 32];
        account_addr.copy_from_slice(&slice.load_bytes(32)?);
        let lt = slice.load_u64()?;
        prev_trans_hash.copy_from_slice(&slice.load_bytes(32)?);
        let prev_trans_lt = slice.load_u64()?;
        let now = slice.load_u32()?;
        let outmsg_cnt = slice.load_uint(OUT_MSG_KEY_BITS)? as u16;
        let orig_status = AccountStatus::load_tlb(slice)?;
        let end_status = AccountStatus::load_tlb(slice)?;

        let messages_cell = slice.load_reference()?;
        let mut messages_slice = Slice::new(messages_cell);
        let in_msg = load_maybe_ref_message(&mut messages_slice)?;
        let out_msgs = messages_slice
            .load_hashmap_e_with(OUT_MSG_KEY_BITS, |slice| {
                load_ref_tlb::<Message>(slice, "Message Any").map_err(anyhow::Error::from)
            })
            .map_err(anyhow_to_tlb_error)?;
        ensure_empty(&messages_slice)?;

        Ok(Self {
            account_addr,
            lt,
            prev_trans_hash,
            prev_trans_lt,
            now,
            outmsg_cnt,
            orig_status,
            end_status,
            in_msg,
            out_msgs,
            total_fees: CurrencyCollection::load_tlb(slice)?,
            state_update: load_ref_tlb(slice, "HASH_UPDATE Account")?,
            description: load_ref_tlb(slice, "TransactionDescr")?,
        })
    }
}

/// TL-B `tr_phase_storage$_ ... = TrStoragePhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrStoragePhase {
    /// Fees collected for storage.
    pub storage_fees_collected: Grams,
    /// Storage fees that remain due, if any.
    pub storage_fees_due: Option<Grams>,
    /// Account status transition caused by storage processing.
    pub status_change: AccStatusChange,
}

impl TlbSerialize for TrStoragePhase {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.storage_fees_collected.store_tlb(builder)?;
        store_maybe(builder, &self.storage_fees_due)?;
        self.status_change.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for TrStoragePhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            storage_fees_collected: Grams::load_tlb(slice)?,
            storage_fees_due: load_maybe(slice)?,
            status_change: AccStatusChange::load_tlb(slice)?,
        })
    }
}

/// TL-B `tr_phase_credit$_ ... = TrCreditPhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrCreditPhase {
    /// Fees collected from the account before crediting, if present.
    pub due_fees_collected: Option<Grams>,
    /// Credited currency collection.
    pub credit: CurrencyCollection,
}

impl TlbSerialize for TrCreditPhase {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_maybe(builder, &self.due_fees_collected)?;
        self.credit.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for TrCreditPhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            due_fees_collected: load_maybe(slice)?,
            credit: CurrencyCollection::load_tlb(slice)?,
        })
    }
}

/// TL-B `ComputeSkipReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputeSkipReason {
    /// `cskip_no_state$00`.
    NoState,
    /// `cskip_bad_state$01`.
    BadState,
    /// `cskip_no_gas$10`.
    NoGas,
    /// `cskip_suspended$110`.
    Suspended,
}

impl TlbSerialize for ComputeSkipReason {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::NoState => store_tag(builder, "00"),
            Self::BadState => store_tag(builder, "01"),
            Self::NoGas => store_tag(builder, "10"),
            Self::Suspended => store_tag(builder, "110"),
        }
    }
}

impl TlbDeserialize for ComputeSkipReason {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = load_tag_bit(slice, "ComputeSkipReason", "00|01|10|110", "")?;
        let second = load_tag_bit(
            slice,
            "ComputeSkipReason",
            "00|01|10|110",
            if first { "1" } else { "0" },
        )?;
        match (first, second) {
            (false, false) => Ok(Self::NoState),
            (false, true) => Ok(Self::BadState),
            (true, false) => Ok(Self::NoGas),
            (true, true) => {
                let third = load_tag_bit(slice, "ComputeSkipReason", "00|01|10|110", "11")?;
                if third {
                    Err(TlbError::TagMismatch {
                        constructor: "ComputeSkipReason",
                        expected_bits: "00|01|10|110",
                        actual_bits: "111".to_string(),
                    })
                } else {
                    Ok(Self::Suspended)
                }
            }
        }
    }
}

/// TL-B `TrComputePhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrComputePhase {
    /// `tr_phase_compute_skipped$0`.
    Skipped {
        /// Reason why VM execution was skipped.
        reason: ComputeSkipReason,
    },
    /// `tr_phase_compute_vm$1`.
    Vm {
        /// Whether VM execution succeeded.
        success: bool,
        /// Whether the incoming message state was used.
        msg_state_used: bool,
        /// Whether the account was activated.
        account_activated: bool,
        /// Gas fees charged for execution.
        gas_fees: Grams,
        /// Gas actually used, encoded as `VarUInteger 7`.
        gas_used: BigUint,
        /// Gas limit, encoded as `VarUInteger 7`.
        gas_limit: BigUint,
        /// Optional gas credit, encoded as `VarUInteger 3`.
        gas_credit: Option<BigUint>,
        /// VM mode byte.
        mode: i8,
        /// VM exit code.
        exit_code: i32,
        /// Optional VM exit argument.
        exit_arg: Option<i32>,
        /// Number of VM steps executed.
        vm_steps: u32,
        /// Initial VM state hash.
        vm_init_state_hash: [u8; 32],
        /// Final VM state hash.
        vm_final_state_hash: [u8; 32],
    },
}

impl TlbSerialize for TrComputePhase {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Skipped { reason } => {
                store_tag(builder, "0")?;
                reason.store_tlb(builder)?;
            }
            Self::Vm {
                success,
                msg_state_used,
                account_activated,
                gas_fees,
                gas_used,
                gas_limit,
                gas_credit,
                mode,
                exit_code,
                exit_arg,
                vm_steps,
                vm_init_state_hash,
                vm_final_state_hash,
            } => {
                store_tag(builder, "1")?;
                builder.store_bit(*success)?;
                builder.store_bit(*msg_state_used)?;
                builder.store_bit(*account_activated)?;
                gas_fees.store_tlb(builder)?;

                let mut child = Builder::new();
                store_var_uint_7(&mut child, gas_used, "TrComputePhase.gas_used")?;
                store_var_uint_7(&mut child, gas_limit, "TrComputePhase.gas_limit")?;
                store_maybe_var_uint_3(&mut child, gas_credit)?;
                child.store_int(*mode as i64, 8)?;
                child.store_int(*exit_code as i64, 32)?;
                store_maybe_i32(&mut child, *exit_arg)?;
                child.store_u32(*vm_steps)?;
                child.store_bytes(vm_init_state_hash)?;
                child.store_bytes(vm_final_state_hash)?;
                builder.store_ref(child.build()?)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for TrComputePhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        if !load_tag_bit(slice, "TrComputePhase", "0|1", "")? {
            return Ok(Self::Skipped {
                reason: ComputeSkipReason::load_tlb(slice)?,
            });
        }

        let success = slice.load_bit()?;
        let msg_state_used = slice.load_bit()?;
        let account_activated = slice.load_bit()?;
        let gas_fees = Grams::load_tlb(slice)?;
        let child = slice.load_reference()?;
        let mut child_slice = Slice::new(child);
        let vm = load_compute_vm_tail(
            &mut child_slice,
            success,
            msg_state_used,
            account_activated,
            gas_fees,
        )
        .map_err(|source| TlbError::InvalidReferencePayload {
            schema: "TrComputePhase.vm",
            source: Box::new(source),
        })?;
        ensure_empty(&child_slice).map_err(|source| TlbError::InvalidReferencePayload {
            schema: "TrComputePhase.vm",
            source: Box::new(source),
        })?;
        Ok(vm)
    }
}

/// TL-B `TrBouncePhase`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrBouncePhase {
    /// `tr_phase_bounce_negfunds$00`.
    NegativeFunds,
    /// `tr_phase_bounce_nofunds$01`.
    NoFunds {
        /// Size of the bounced message.
        msg_size: StorageUsed,
        /// Required forwarding fees.
        req_fwd_fees: Grams,
    },
    /// `tr_phase_bounce_ok$1`.
    Ok {
        /// Size of the bounced message.
        msg_size: StorageUsed,
        /// Message fees.
        msg_fees: Grams,
        /// Forwarding fees.
        fwd_fees: Grams,
    },
}

impl TlbSerialize for TrBouncePhase {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::NegativeFunds => store_tag(builder, "00")?,
            Self::NoFunds {
                msg_size,
                req_fwd_fees,
            } => {
                store_tag(builder, "01")?;
                msg_size.store_tlb(builder)?;
                req_fwd_fees.store_tlb(builder)?;
            }
            Self::Ok {
                msg_size,
                msg_fees,
                fwd_fees,
            } => {
                store_tag(builder, "1")?;
                msg_size.store_tlb(builder)?;
                msg_fees.store_tlb(builder)?;
                fwd_fees.store_tlb(builder)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for TrBouncePhase {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let first = load_tag_bit(slice, "TrBouncePhase", "00|01|1", "")?;
        if first {
            return Ok(Self::Ok {
                msg_size: StorageUsed::load_tlb(slice)?,
                msg_fees: Grams::load_tlb(slice)?,
                fwd_fees: Grams::load_tlb(slice)?,
            });
        }

        let second = load_tag_bit(slice, "TrBouncePhase", "00|01|1", "0")?;
        if second {
            Ok(Self::NoFunds {
                msg_size: StorageUsed::load_tlb(slice)?,
                req_fwd_fees: Grams::load_tlb(slice)?,
            })
        } else {
            Ok(Self::NegativeFunds)
        }
    }
}

/// TL-B `split_merge_info$_ ... = SplitMergeInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMergeInfo {
    /// Current shard prefix length, encoded in six bits.
    pub cur_shard_pfx_len: u8,
    /// Account split depth, encoded in six bits.
    pub acc_split_depth: u8,
    /// Current account address bits.
    pub this_addr: [u8; 32],
    /// Sibling account address bits.
    pub sibling_addr: [u8; 32],
}

impl TlbSerialize for SplitMergeInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        validate_u6("SplitMergeInfo.cur_shard_pfx_len", self.cur_shard_pfx_len)?;
        validate_u6("SplitMergeInfo.acc_split_depth", self.acc_split_depth)?;
        builder.store_uint(self.cur_shard_pfx_len as u64, 6)?;
        builder.store_uint(self.acc_split_depth as u64, 6)?;
        builder.store_bytes(&self.this_addr)?;
        builder.store_bytes(&self.sibling_addr)?;
        Ok(())
    }
}

impl TlbDeserialize for SplitMergeInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let mut this_addr = [0; 32];
        let mut sibling_addr = [0; 32];
        let cur_shard_pfx_len = slice.load_uint(6)? as u8;
        let acc_split_depth = slice.load_uint(6)? as u8;
        this_addr.copy_from_slice(&slice.load_bytes(32)?);
        sibling_addr.copy_from_slice(&slice.load_bytes(32)?);
        Ok(Self {
            cur_shard_pfx_len,
            acc_split_depth,
            this_addr,
            sibling_addr,
        })
    }
}

/// TL-B `TransactionDescr` constructors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionDescr {
    /// `trans_ord$0000`.
    Ordinary {
        /// Whether credit is processed before storage.
        credit_first: bool,
        /// Optional storage phase.
        storage_ph: Option<TrStoragePhase>,
        /// Optional credit phase.
        credit_ph: Option<TrCreditPhase>,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Optional bounce phase.
        bounce: Option<TrBouncePhase>,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
    /// `trans_storage$0001`.
    Storage {
        /// Storage-only phase.
        storage_ph: TrStoragePhase,
    },
    /// `trans_tick_tock$001`.
    TickTock {
        /// `true` for tock, `false` for tick.
        is_tock: bool,
        /// Storage phase.
        storage_ph: TrStoragePhase,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
    /// `trans_split_prepare$0100`.
    SplitPrepare {
        /// Split metadata.
        split_info: SplitMergeInfo,
        /// Optional storage phase.
        storage_ph: Option<TrStoragePhase>,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
    /// `trans_split_install$0101`.
    SplitInstall {
        /// Split metadata.
        split_info: SplitMergeInfo,
        /// Referenced prepared transaction.
        prepare_transaction: Box<Transaction>,
        /// Whether the prepared transaction was installed.
        installed: bool,
    },
    /// `trans_merge_prepare$0110`.
    MergePrepare {
        /// Merge metadata.
        split_info: SplitMergeInfo,
        /// Storage phase.
        storage_ph: TrStoragePhase,
        /// Whether the transaction aborted.
        aborted: bool,
    },
    /// `trans_merge_install$0111`.
    MergeInstall {
        /// Merge metadata.
        split_info: SplitMergeInfo,
        /// Referenced prepared transaction.
        prepare_transaction: Box<Transaction>,
        /// Optional storage phase.
        storage_ph: Option<TrStoragePhase>,
        /// Optional credit phase.
        credit_ph: Option<TrCreditPhase>,
        /// Compute phase.
        compute_ph: TrComputePhase,
        /// Optional referenced action phase.
        action: Option<TrActionPhase>,
        /// Whether the transaction aborted.
        aborted: bool,
        /// Whether the account was destroyed.
        destroyed: bool,
    },
}

impl TlbSerialize for TransactionDescr {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Ordinary {
                credit_first,
                storage_ph,
                credit_ph,
                compute_ph,
                action,
                aborted,
                bounce,
                destroyed,
            } => {
                store_tag(builder, "0000")?;
                builder.store_bit(*credit_first)?;
                store_maybe(builder, storage_ph)?;
                store_maybe(builder, credit_ph)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                store_maybe(builder, bounce)?;
                builder.store_bit(*destroyed)?;
            }
            Self::Storage { storage_ph } => {
                store_tag(builder, "0001")?;
                storage_ph.store_tlb(builder)?;
            }
            Self::TickTock {
                is_tock,
                storage_ph,
                compute_ph,
                action,
                aborted,
                destroyed,
            } => {
                store_tag(builder, "001")?;
                builder.store_bit(*is_tock)?;
                storage_ph.store_tlb(builder)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                builder.store_bit(*destroyed)?;
            }
            Self::SplitPrepare {
                split_info,
                storage_ph,
                compute_ph,
                action,
                aborted,
                destroyed,
            } => {
                store_tag(builder, "0100")?;
                split_info.store_tlb(builder)?;
                store_maybe(builder, storage_ph)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                builder.store_bit(*destroyed)?;
            }
            Self::SplitInstall {
                split_info,
                prepare_transaction,
                installed,
            } => {
                store_tag(builder, "0101")?;
                split_info.store_tlb(builder)?;
                store_ref_tlb(builder, prepare_transaction.as_ref())?;
                builder.store_bit(*installed)?;
            }
            Self::MergePrepare {
                split_info,
                storage_ph,
                aborted,
            } => {
                store_tag(builder, "0110")?;
                split_info.store_tlb(builder)?;
                storage_ph.store_tlb(builder)?;
                builder.store_bit(*aborted)?;
            }
            Self::MergeInstall {
                split_info,
                prepare_transaction,
                storage_ph,
                credit_ph,
                compute_ph,
                action,
                aborted,
                destroyed,
            } => {
                store_tag(builder, "0111")?;
                split_info.store_tlb(builder)?;
                store_ref_tlb(builder, prepare_transaction.as_ref())?;
                store_maybe(builder, storage_ph)?;
                store_maybe(builder, credit_ph)?;
                compute_ph.store_tlb(builder)?;
                store_maybe_ref_action_phase(builder, action)?;
                builder.store_bit(*aborted)?;
                builder.store_bit(*destroyed)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for TransactionDescr {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        match load_transaction_descr_tag(slice)? {
            TransactionDescrTag::Ordinary => Ok(Self::Ordinary {
                credit_first: slice.load_bit()?,
                storage_ph: load_maybe(slice)?,
                credit_ph: load_maybe(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                bounce: load_maybe(slice)?,
                destroyed: slice.load_bit()?,
            }),
            TransactionDescrTag::Storage => Ok(Self::Storage {
                storage_ph: TrStoragePhase::load_tlb(slice)?,
            }),
            TransactionDescrTag::TickTock => Ok(Self::TickTock {
                is_tock: slice.load_bit()?,
                storage_ph: TrStoragePhase::load_tlb(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                destroyed: slice.load_bit()?,
            }),
            TransactionDescrTag::SplitPrepare => Ok(Self::SplitPrepare {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                storage_ph: load_maybe(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                destroyed: slice.load_bit()?,
            }),
            TransactionDescrTag::SplitInstall => Ok(Self::SplitInstall {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                prepare_transaction: Box::new(load_ref_tlb(slice, "Transaction")?),
                installed: slice.load_bit()?,
            }),
            TransactionDescrTag::MergePrepare => Ok(Self::MergePrepare {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                storage_ph: TrStoragePhase::load_tlb(slice)?,
                aborted: slice.load_bit()?,
            }),
            TransactionDescrTag::MergeInstall => Ok(Self::MergeInstall {
                split_info: SplitMergeInfo::load_tlb(slice)?,
                prepare_transaction: Box::new(load_ref_tlb(slice, "Transaction")?),
                storage_ph: load_maybe(slice)?,
                credit_ph: load_maybe(slice)?,
                compute_ph: TrComputePhase::load_tlb(slice)?,
                action: load_maybe_ref_action_phase(slice)?,
                aborted: slice.load_bit()?,
                destroyed: slice.load_bit()?,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionDescrTag {
    Ordinary,
    Storage,
    TickTock,
    SplitPrepare,
    SplitInstall,
    MergePrepare,
    MergeInstall,
}

fn load_compute_vm_tail(
    slice: &mut Slice,
    success: bool,
    msg_state_used: bool,
    account_activated: bool,
    gas_fees: Grams,
) -> Result<TrComputePhase> {
    let gas_used = load_var_uint_7(slice, "TrComputePhase.gas_used")?;
    let gas_limit = load_var_uint_7(slice, "TrComputePhase.gas_limit")?;
    let gas_credit = load_maybe_var_uint_3(slice)?;
    let mode = slice.load_int(8)? as i8;
    let exit_code = slice.load_int(32)? as i32;
    let exit_arg = load_maybe_i32(slice)?;
    let vm_steps = slice.load_u32()?;
    let mut vm_init_state_hash = [0; 32];
    vm_init_state_hash.copy_from_slice(&slice.load_bytes(32)?);
    let mut vm_final_state_hash = [0; 32];
    vm_final_state_hash.copy_from_slice(&slice.load_bytes(32)?);
    Ok(TrComputePhase::Vm {
        success,
        msg_state_used,
        account_activated,
        gas_fees,
        gas_used,
        gas_limit,
        gas_credit,
        mode,
        exit_code,
        exit_arg,
        vm_steps,
        vm_init_state_hash,
        vm_final_state_hash,
    })
}

fn store_maybe_ref_action_phase(
    builder: &mut Builder,
    action: &Option<TrActionPhase>,
) -> Result<()> {
    match action {
        Some(action) => {
            builder.store_bit(true)?;
            store_ref_tlb(builder, action)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_ref_action_phase(slice: &mut Slice) -> Result<Option<TrActionPhase>> {
    if slice.load_bit()? {
        Ok(Some(load_ref_tlb(slice, "TrActionPhase")?))
    } else {
        Ok(None)
    }
}

fn store_maybe_i32(builder: &mut Builder, value: Option<i32>) -> Result<()> {
    match value {
        Some(value) => {
            builder.store_bit(true)?;
            builder.store_int(value as i64, 32)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_i32(slice: &mut Slice) -> Result<Option<i32>> {
    if slice.load_bit()? {
        Ok(Some(slice.load_int(32)? as i32))
    } else {
        Ok(None)
    }
}

fn store_maybe_var_uint_3(builder: &mut Builder, value: &Option<BigUint>) -> Result<()> {
    match value {
        Some(value) => {
            builder.store_bit(true)?;
            store_var_uint_3(builder, value, "TrComputePhase.gas_credit")?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_var_uint_3(slice: &mut Slice) -> Result<Option<BigUint>> {
    if slice.load_bit()? {
        Ok(Some(load_var_uint_3(slice, "TrComputePhase.gas_credit")?))
    } else {
        Ok(None)
    }
}

fn store_var_uint_7(builder: &mut Builder, value: &BigUint, schema: &'static str) -> Result<()> {
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_7_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_7_MAX_BYTES}"),
        });
    }
    store_var_uint(builder, value, VAR_UINT_7_LEN_BITS)
}

fn load_var_uint_7(slice: &mut Slice, schema: &'static str) -> Result<BigUint> {
    let value = load_var_uint(slice, VAR_UINT_7_LEN_BITS)?;
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_7_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_7_MAX_BYTES}"),
        });
    }
    Ok(value)
}

fn store_var_uint_3(builder: &mut Builder, value: &BigUint, schema: &'static str) -> Result<()> {
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_3_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_3_MAX_BYTES}"),
        });
    }
    store_var_uint(builder, value, VAR_UINT_3_LEN_BITS)
}

fn load_var_uint_3(slice: &mut Slice, schema: &'static str) -> Result<BigUint> {
    let value = load_var_uint(slice, VAR_UINT_3_LEN_BITS)?;
    let byte_len = value.to_bytes_be().len();
    if byte_len > VAR_UINT_3_MAX_BYTES {
        return Err(TlbError::NonCanonicalValue {
            schema,
            reason: format!("byte length {byte_len} exceeds maximum {VAR_UINT_3_MAX_BYTES}"),
        });
    }
    Ok(value)
}

fn validate_u6(schema: &'static str, value: u8) -> Result<()> {
    if value > 63 {
        Err(TlbError::CustomSchema {
            schema,
            message: format!("value {value} does not fit in six bits"),
        })
    } else {
        Ok(())
    }
}

fn store_maybe_ref_message(builder: &mut Builder, message: &Option<Message>) -> Result<()> {
    match message {
        Some(message) => {
            builder.store_bit(true)?;
            store_ref_tlb(builder, message)?;
        }
        None => {
            builder.store_bit(false)?;
        }
    }
    Ok(())
}

fn load_maybe_ref_message(slice: &mut Slice) -> Result<Option<Message>> {
    if slice.load_bit()? {
        Ok(Some(load_ref_tlb(slice, "Message Any")?))
    } else {
        Ok(None)
    }
}

fn expect_tag_bits(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<()> {
    let mut actual_bits = String::with_capacity(expected_bits.len());
    for expected in expected_bits.bytes() {
        let bit = slice.load_bit().map_err(|_| TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits: actual_bits.clone(),
        })?;
        actual_bits.push(if bit { '1' } else { '0' });
        if bit != (expected == b'1') {
            return Err(TlbError::TagMismatch {
                constructor,
                expected_bits,
                actual_bits,
            });
        }
    }
    Ok(())
}

fn expect_u8_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
    expected_tag: u8,
) -> Result<()> {
    let mut actual_bits = String::with_capacity(8);
    let mut tag = 0u8;
    for _ in 0..8 {
        let bit = slice.load_bit().map_err(|_| TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits: actual_bits.clone(),
        })?;
        actual_bits.push(if bit { '1' } else { '0' });
        tag = (tag << 1) | u8::from(bit);
    }
    if tag == expected_tag {
        Ok(())
    } else {
        Err(TlbError::TagMismatch {
            constructor,
            expected_bits,
            actual_bits,
        })
    }
}

fn load_two_bit_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<String> {
    let mut actual_bits = String::with_capacity(2);
    for _ in 0..2 {
        match slice.load_bit() {
            Ok(bit) => actual_bits.push(if bit { '1' } else { '0' }),
            Err(_) => {
                return Err(TlbError::TagMismatch {
                    constructor,
                    expected_bits,
                    actual_bits,
                });
            }
        }
    }
    Ok(actual_bits)
}

fn load_three_bit_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<String> {
    let mut actual_bits = String::with_capacity(3);
    for _ in 0..3 {
        match slice.load_bit() {
            Ok(bit) => actual_bits.push(if bit { '1' } else { '0' }),
            Err(_) => {
                return Err(TlbError::TagMismatch {
                    constructor,
                    expected_bits,
                    actual_bits,
                });
            }
        }
    }
    Ok(actual_bits)
}

fn anyhow_to_tlb_error(error: anyhow::Error) -> TlbError {
    match error.downcast::<TlbError>() {
        Ok(error) => error,
        Err(error) => TlbError::Tvm(error),
    }
}

fn load_tag_bit(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
    actual_prefix: &'static str,
) -> Result<bool> {
    slice.load_bit().map_err(|_| TlbError::TagMismatch {
        constructor,
        expected_bits,
        actual_bits: actual_prefix.to_string(),
    })
}

fn load_transaction_descr_tag(slice: &mut Slice) -> Result<TransactionDescrTag> {
    let b0 = load_descr_tag_bit(slice, "")?;
    if b0 {
        return Err(TlbError::TagMismatch {
            constructor: "TransactionDescr",
            expected_bits: "0000|0001|001|0100|0101|0110|0111",
            actual_bits: "1".to_string(),
        });
    }

    let b1 = load_descr_tag_bit(slice, "0")?;
    let b2 = load_descr_tag_bit(slice, if b1 { "01" } else { "00" })?;
    match (b1, b2) {
        (false, false) => match load_descr_tag_bit(slice, "000")? {
            false => Ok(TransactionDescrTag::Ordinary),
            true => Ok(TransactionDescrTag::Storage),
        },
        (false, true) => Ok(TransactionDescrTag::TickTock),
        (true, false) => match load_descr_tag_bit(slice, "010")? {
            false => Ok(TransactionDescrTag::SplitPrepare),
            true => Ok(TransactionDescrTag::SplitInstall),
        },
        (true, true) => match load_descr_tag_bit(slice, "011")? {
            false => Ok(TransactionDescrTag::MergePrepare),
            true => Ok(TransactionDescrTag::MergeInstall),
        },
    }
}

fn load_descr_tag_bit(slice: &mut Slice, actual_prefix: &'static str) -> Result<bool> {
    load_tag_bit(
        slice,
        "TransactionDescr",
        "0000|0001|001|0100|0101|0110|0111",
        actual_prefix,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tlb::{CommonMsgInfo, Either, TlbSerialize, store_tag};
    use crate::tvm::{Address, BitKey, Cell, HashmapAug, HashmapAugE, HashmapAugLeaf};
    use std::sync::Arc;

    fn roundtrip<T>(value: &T) -> T
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + std::fmt::Debug,
    {
        T::from_cell(value.to_cell().unwrap()).unwrap()
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn storage_phase() -> TrStoragePhase {
        TrStoragePhase {
            storage_fees_collected: Grams::from(7),
            storage_fees_due: Some(Grams::from(8)),
            status_change: AccStatusChange::Frozen,
        }
    }

    fn credit_phase() -> TrCreditPhase {
        TrCreditPhase {
            due_fees_collected: Some(Grams::from(1)),
            credit: CurrencyCollection::grams(Grams::from(10)),
        }
    }

    fn compute_skipped() -> TrComputePhase {
        TrComputePhase::Skipped {
            reason: ComputeSkipReason::NoGas,
        }
    }

    fn compute_vm() -> TrComputePhase {
        TrComputePhase::Vm {
            success: true,
            msg_state_used: false,
            account_activated: true,
            gas_fees: Grams::from(11),
            gas_used: BigUint::from(12u8),
            gas_limit: BigUint::from(13u8),
            gas_credit: Some(BigUint::from(2u8)),
            mode: -1,
            exit_code: -14,
            exit_arg: Some(32),
            vm_steps: 1234,
            vm_init_state_hash: [0x11; 32],
            vm_final_state_hash: [0x22; 32],
        }
    }

    fn action_phase() -> TrActionPhase {
        TrActionPhase {
            success: true,
            valid: true,
            no_funds: false,
            status_change: AccStatusChange::Unchanged,
            total_fwd_fees: Some(Grams::from(3)),
            total_action_fees: None,
            result_code: 0,
            result_arg: None,
            tot_actions: 1,
            spec_actions: 0,
            skipped_actions: 0,
            msgs_created: 1,
            action_list_hash: [0x33; 32],
            tot_msg_size: StorageUsed::new(BigUint::from(1u8), BigUint::from(64u8)),
        }
    }

    fn split_info() -> SplitMergeInfo {
        SplitMergeInfo {
            cur_shard_pfx_len: 12,
            acc_split_depth: 6,
            this_addr: [0x44; 32],
            sibling_addr: [0x55; 32],
        }
    }

    fn account_address() -> MsgAddressInt {
        MsgAddressInt::std(Address::new(0, [0x11; 32]))
    }

    fn message() -> Message {
        Message {
            info: CommonMsgInfo::ExternalIn {
                src: crate::tlb::MsgAddressExt::None,
                dest: account_address(),
                import_fee: Grams::from(1),
            },
            init: None,
            body: Either::Right(cell_with_bits(&[0x80], 1)),
        }
    }

    fn hash_update() -> HashUpdateAccount {
        HashUpdateAccount {
            old_hash: [0xAA; 32],
            new_hash: [0xBB; 32],
        }
    }

    fn storage_info() -> StorageInfo {
        StorageInfo {
            used: StorageUsed::new(BigUint::from(2u8), BigUint::from(128u16)),
            last_paid: 1_700_000_001,
            due_payment: Some(Grams::from(4)),
            extra: StorageExtraInfo::Info {
                dict_hash: [0xCC; 32],
            },
        }
    }

    fn account_storage() -> AccountStorage {
        AccountStorage {
            last_trans_lt: 11,
            balance: CurrencyCollection::grams(Grams::from(100)),
            state: AccountState::Active {
                state_init: StateInit::empty(),
            },
        }
    }

    fn account() -> Account {
        Account::Full {
            addr: account_address(),
            storage_stat: storage_info(),
            storage: account_storage(),
        }
    }

    fn simple_transaction() -> Transaction {
        Transaction {
            account_addr: [0x10; 32],
            lt: 7,
            prev_trans_hash: [0x20; 32],
            prev_trans_lt: 6,
            now: 1_700_000_000,
            outmsg_cnt: 0,
            orig_status: AccountStatus::Active,
            end_status: AccountStatus::Active,
            in_msg: None,
            out_msgs: HashmapE::new(OUT_MSG_KEY_BITS),
            total_fees: CurrencyCollection::grams(Grams::from(3)),
            state_update: hash_update(),
            description: TransactionDescr::Storage {
                storage_ph: storage_phase(),
            },
        }
    }

    fn transaction_with_messages() -> Transaction {
        let mut out_msgs = HashmapE::new(OUT_MSG_KEY_BITS);
        out_msgs
            .insert_bit_key(BitKey::from_u64(3, OUT_MSG_KEY_BITS).unwrap(), message())
            .unwrap();
        Transaction {
            in_msg: Some(message()),
            outmsg_cnt: 1,
            out_msgs,
            ..simple_transaction()
        }
    }

    fn depth_balance(split_depth: u8, grams: u64) -> DepthBalanceInfo {
        DepthBalanceInfo {
            split_depth,
            balance: CurrencyCollection::grams(Grams::from(grams)),
        }
    }

    fn account_block_with_lts(lts: &[u64]) -> AccountBlock {
        let entries = lts
            .iter()
            .copied()
            .map(|lt| HashmapAugLeaf {
                key: BitKey::from_u64(lt, 64).unwrap(),
                value: Transaction {
                    lt,
                    ..simple_transaction()
                },
                extra: CurrencyCollection::grams(Grams::from(lt + 1)),
            })
            .collect();
        AccountBlock {
            account_addr: [0x55; 32],
            transactions: HashmapAug::from_entries(
                64,
                entries,
                CurrencyCollection::grams(Grams::from(99)),
            )
            .unwrap(),
            state_update: hash_update(),
        }
    }

    fn store_transaction_prefix(builder: &mut Builder) {
        store_tag(builder, "0111").unwrap();
        builder.store_bytes(&[0x10; 32]).unwrap();
        builder.store_u64(7).unwrap();
        builder.store_bytes(&[0x20; 32]).unwrap();
        builder.store_u64(6).unwrap();
        builder.store_u32(1_700_000_000).unwrap();
        builder.store_uint(0, OUT_MSG_KEY_BITS).unwrap();
        AccountStatus::Active.store_tlb(builder).unwrap();
        AccountStatus::Active.store_tlb(builder).unwrap();
    }

    fn store_transaction_suffix(builder: &mut Builder) {
        CurrencyCollection::grams(Grams::from(3))
            .store_tlb(builder)
            .unwrap();
        store_ref_tlb(builder, &hash_update()).unwrap();
        store_ref_tlb(
            builder,
            &TransactionDescr::Storage {
                storage_ph: storage_phase(),
            },
        )
        .unwrap();
    }

    #[test]
    fn storage_and_credit_phases_roundtrip() {
        assert_eq!(roundtrip(&storage_phase()), storage_phase());
        assert_eq!(roundtrip(&credit_phase()), credit_phase());
    }

    #[test]
    fn storage_extra_info_roundtrips() {
        assert_eq!(roundtrip(&StorageExtraInfo::None), StorageExtraInfo::None);
        let value = StorageExtraInfo::Info {
            dict_hash: [0x12; 32],
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn storage_extra_info_and_storage_info_match_upstream_layout() {
        let none = StorageExtraInfo::None.to_cell().unwrap();
        assert_eq!(none.bit_len(), 3);
        assert_eq!(none.data()[0] >> 5, 0);

        let info = StorageExtraInfo::Info {
            dict_hash: [0x12; 32],
        }
        .to_cell()
        .unwrap();
        assert_eq!(info.bit_len(), 259);
        assert_eq!(info.data()[0] >> 5, 1);

        let value = storage_info();
        let mut slice = Slice::new(value.to_cell().unwrap());
        assert_eq!(StorageUsed::load_tlb(&mut slice).unwrap(), value.used);
        assert_eq!(slice.load_bits(3).unwrap(), vec![0b0010_0000]);
        let mut dict_hash = [0; 32];
        dict_hash.copy_from_slice(&slice.load_bytes(32).unwrap());
        assert_eq!(dict_hash, [0xCC; 32]);
        assert_eq!(slice.load_u32().unwrap(), value.last_paid);
        assert_eq!(load_maybe::<Grams>(&mut slice).unwrap(), value.due_payment);
    }

    #[test]
    fn account_state_all_tags_roundtrip() {
        let values = [
            AccountState::Uninit,
            AccountState::Frozen {
                state_hash: [0x34; 32],
            },
            AccountState::Active {
                state_init: StateInit::empty(),
            },
        ];
        for value in values {
            assert_eq!(roundtrip(&value), value);
        }
    }

    #[test]
    fn account_status_all_tags_roundtrip() {
        for value in [
            AccountStatus::Uninit,
            AccountStatus::Frozen,
            AccountStatus::Active,
            AccountStatus::Nonexist,
        ] {
            assert_eq!(roundtrip(&value), value);
        }
    }

    #[test]
    fn account_and_shard_account_roundtrip() {
        assert_eq!(roundtrip(&Account::None), Account::None);
        assert_eq!(roundtrip(&account()), account());

        let shard = ShardAccount {
            account: account(),
            last_trans_hash: [0x44; 32],
            last_trans_lt: 12,
        };
        assert_eq!(roundtrip(&shard), shard);
    }

    #[test]
    fn depth_balance_and_augmented_account_collections_roundtrip() {
        assert_eq!(roundtrip(&depth_balance(30, 123)), depth_balance(30, 123));

        let shard_account = ShardAccount {
            account: account(),
            last_trans_hash: [0x44; 32],
            last_trans_lt: 12,
        };
        let shard_root = HashmapAug::from_entries(
            256,
            vec![HashmapAugLeaf {
                key: BitKey::from_bits(vec![0x11; 32], 256).unwrap(),
                value: shard_account,
                extra: depth_balance(7, 100),
            }],
            depth_balance(7, 100),
        )
        .unwrap();
        let shard_accounts = ShardAccounts {
            accounts: HashmapAugE::with_root(256, shard_root, depth_balance(7, 100)).unwrap(),
        };
        assert_eq!(roundtrip(&shard_accounts), shard_accounts);

        let blocks_root = HashmapAug::from_entries(
            256,
            vec![HashmapAugLeaf {
                key: BitKey::from_bits(vec![0x22; 32], 256).unwrap(),
                value: account_block_with_lts(&[7]),
                extra: CurrencyCollection::grams(Grams::from(8)),
            }],
            CurrencyCollection::grams(Grams::from(8)),
        )
        .unwrap();
        let blocks = ShardAccountBlocks {
            blocks: HashmapAugE::with_root(
                256,
                blocks_root,
                CurrencyCollection::grams(Grams::from(8)),
            )
            .unwrap(),
        };
        assert_eq!(roundtrip(&blocks), blocks);
    }

    #[test]
    fn account_block_roundtrips_with_one_and_multiple_transactions() {
        assert_eq!(
            roundtrip(&account_block_with_lts(&[7])),
            account_block_with_lts(&[7])
        );
        assert_eq!(
            roundtrip(&account_block_with_lts(&[7, 9])),
            account_block_with_lts(&[7, 9])
        );
    }

    #[test]
    fn depth_balance_rejects_split_depth_above_30() {
        let err = depth_balance(31, 1).to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "DepthBalanceInfo.split_depth",
                ..
            }
        ));

        let mut builder = Builder::new();
        builder.store_uint(31, 5).unwrap();
        CurrencyCollection::grams(Grams::from(1))
            .store_tlb(&mut builder)
            .unwrap();
        let err = DepthBalanceInfo::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::NonCanonicalValue {
                schema: "DepthBalanceInfo.split_depth",
                ..
            }
        ));
    }

    #[test]
    fn account_block_reports_malformed_references() {
        let bad_ref_entries = vec![HashmapAugLeaf {
            key: BitKey::from_u64(7, 64).unwrap(),
            value: cell_with_bits(&[0x80], 1),
            extra: CurrencyCollection::grams(Grams::from(1)),
        }];
        let bad_ref_dict = HashmapAug::from_entries(
            64,
            bad_ref_entries,
            CurrencyCollection::grams(Grams::from(1)),
        )
        .unwrap();
        let mut builder = Builder::new();
        store_tag(&mut builder, "0101").unwrap();
        builder.store_bytes(&[0x55; 32]).unwrap();
        builder
            .store_hashmap_aug_with(
                &bad_ref_dict,
                |builder, cell| {
                    builder.store_ref(cell.clone())?;
                    Ok(())
                },
                |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
            )
            .unwrap();
        store_ref_tlb(&mut builder, &hash_update()).unwrap();
        let err = AccountBlock::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "Transaction",
                ..
            }
        ));

        let mut builder = Builder::new();
        store_tag(&mut builder, "0101").unwrap();
        builder.store_bytes(&[0x55; 32]).unwrap();
        builder
            .store_hashmap_aug_with(
                &account_block_with_lts(&[7]).transactions,
                |builder, transaction| {
                    store_ref_tlb(builder, transaction).map_err(anyhow::Error::from)
                },
                |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
            )
            .unwrap();
        builder.store_ref(cell_with_bits(&[0x71], 8)).unwrap();
        let err = AccountBlock::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "HASH_UPDATE Account",
                ..
            }
        ));
    }

    #[test]
    fn exact_account_block_decode_rejects_trailing_data() {
        let mut builder = Builder::new();
        account_block_with_lts(&[7])
            .store_tlb(&mut builder)
            .unwrap();
        builder.store_bit(true).unwrap();
        let err = AccountBlock::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
    }

    #[test]
    fn hash_update_account_roundtrips() {
        assert_eq!(roundtrip(&hash_update()), hash_update());
    }

    #[test]
    fn transaction_roundtrips_without_and_with_messages() {
        assert_eq!(roundtrip(&simple_transaction()), simple_transaction());
        assert_eq!(
            roundtrip(&transaction_with_messages()),
            transaction_with_messages()
        );
    }

    #[test]
    fn transaction_serialization_rejects_outmsg_count_above_uint15() {
        let value = Transaction {
            outmsg_cnt: 0x8000,
            ..simple_transaction()
        };
        let err = value.to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "Transaction.outmsg_cnt",
                ..
            }
        ));
    }

    #[test]
    fn transaction_serialization_rejects_non_15_bit_out_msg_dictionary() {
        let value = Transaction {
            out_msgs: HashmapE::new(16),
            ..simple_transaction()
        };
        let err = value.to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "Transaction.out_msgs",
                ..
            }
        ));
    }

    #[test]
    fn truncated_new_constructor_tags_are_rejected() {
        let err = Account::from_cell(Builder::new().build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::Tvm(_)));

        let err = AccountState::from_cell(cell_with_bits(&[0], 1)).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "AccountState",
                actual_bits,
                ..
            } if actual_bits == "0"
        ));

        let err = AccountStatus::from_cell(Builder::new().build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "AccountStatus",
                actual_bits,
                ..
            } if actual_bits.is_empty()
        ));

        let err = HashUpdateAccount::from_cell(cell_with_bits(&[0x71], 8)).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "HASH_UPDATE Account",
                ..
            }
        ));

        let err = HashUpdateAccount::from_cell(cell_with_bits(&[0x70], 4)).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "HASH_UPDATE Account",
                actual_bits,
                ..
            } if actual_bits == "0111"
        ));

        let err = Transaction::from_cell(cell_with_bits(&[0x80], 1)).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "Transaction",
                actual_bits,
                ..
            } if actual_bits == "1"
        ));
    }

    #[test]
    fn malformed_transaction_message_reference_is_reported() {
        let mut messages = Builder::new();
        messages.store_bit(true).unwrap();
        messages.store_ref(cell_with_bits(&[0x80], 1)).unwrap();
        messages.store_bit(false).unwrap();

        let mut builder = Builder::new();
        store_transaction_prefix(&mut builder);
        builder.store_ref(messages.build().unwrap()).unwrap();
        store_transaction_suffix(&mut builder);

        let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "Message Any",
                ..
            }
        ));
    }

    #[test]
    fn malformed_transaction_out_message_reference_is_reported() {
        let mut out_msgs = HashmapE::new(OUT_MSG_KEY_BITS);
        out_msgs
            .insert_bit_key(
                BitKey::from_u64(1, OUT_MSG_KEY_BITS).unwrap(),
                cell_with_bits(&[0x80], 1),
            )
            .unwrap();

        let mut messages = Builder::new();
        messages.store_bit(false).unwrap();
        messages
            .store_hashmap_e_with(&out_msgs, |builder, cell| {
                builder.store_ref(cell.clone())?;
                Ok(())
            })
            .unwrap();

        let mut builder = Builder::new();
        store_transaction_prefix(&mut builder);
        builder.store_ref(messages.build().unwrap()).unwrap();
        store_transaction_suffix(&mut builder);

        let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "Message Any",
                ..
            }
        ));
    }

    #[test]
    fn malformed_transaction_state_update_and_description_references_are_reported() {
        let mut messages = Builder::new();
        messages.store_bit(false).unwrap();
        messages.store_bit(false).unwrap();

        let mut builder = Builder::new();
        store_transaction_prefix(&mut builder);
        builder.store_ref(messages.build().unwrap()).unwrap();
        CurrencyCollection::grams(Grams::from(3))
            .store_tlb(&mut builder)
            .unwrap();
        builder.store_ref(cell_with_bits(&[0x71], 8)).unwrap();
        store_ref_tlb(
            &mut builder,
            &TransactionDescr::Storage {
                storage_ph: storage_phase(),
            },
        )
        .unwrap();
        let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "HASH_UPDATE Account",
                ..
            }
        ));

        let mut messages = Builder::new();
        messages.store_bit(false).unwrap();
        messages.store_bit(false).unwrap();
        let mut builder = Builder::new();
        store_transaction_prefix(&mut builder);
        builder.store_ref(messages.build().unwrap()).unwrap();
        CurrencyCollection::grams(Grams::from(3))
            .store_tlb(&mut builder)
            .unwrap();
        store_ref_tlb(&mut builder, &hash_update()).unwrap();
        builder.store_ref(cell_with_bits(&[0x80], 1)).unwrap();
        let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "TransactionDescr",
                ..
            }
        ));
    }

    #[test]
    fn exact_transaction_decode_rejects_trailing_data() {
        let mut builder = Builder::new();
        simple_transaction().store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));

        let mut builder = Builder::new();
        simple_transaction().store_tlb(&mut builder).unwrap();
        builder.store_ref(cell_with_bits(&[0x80], 1)).unwrap();
        let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 0, refs: 1 }));
    }

    #[test]
    fn compute_skip_reason_all_tags_roundtrip() {
        for reason in [
            ComputeSkipReason::NoState,
            ComputeSkipReason::BadState,
            ComputeSkipReason::NoGas,
            ComputeSkipReason::Suspended,
        ] {
            assert_eq!(roundtrip(&reason), reason);
        }
    }

    #[test]
    fn compute_phases_roundtrip() {
        assert_eq!(roundtrip(&compute_skipped()), compute_skipped());
        assert_eq!(roundtrip(&compute_vm()), compute_vm());
    }

    #[test]
    fn bounce_phase_all_tags_roundtrip() {
        let msg_size = StorageUsed::new(BigUint::from(2u8), BigUint::from(16u8));
        let values = [
            TrBouncePhase::NegativeFunds,
            TrBouncePhase::NoFunds {
                msg_size: msg_size.clone(),
                req_fwd_fees: Grams::from(9),
            },
            TrBouncePhase::Ok {
                msg_size,
                msg_fees: Grams::from(10),
                fwd_fees: Grams::from(11),
            },
        ];
        for value in values {
            assert_eq!(roundtrip(&value), value);
        }
    }

    #[test]
    fn split_merge_info_roundtrips_and_rejects_out_of_range_values() {
        assert_eq!(roundtrip(&split_info()), split_info());

        let invalid = SplitMergeInfo {
            cur_shard_pfx_len: 64,
            ..split_info()
        };
        let err = invalid.to_cell().unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "SplitMergeInfo.cur_shard_pfx_len",
                ..
            }
        ));
    }

    #[test]
    fn ordinary_description_roundtrips_without_action() {
        let value = TransactionDescr::Ordinary {
            credit_first: true,
            storage_ph: Some(storage_phase()),
            credit_ph: Some(credit_phase()),
            compute_ph: compute_skipped(),
            action: None,
            aborted: false,
            bounce: Some(TrBouncePhase::NegativeFunds),
            destroyed: false,
        };

        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn ordinary_description_roundtrips_with_referenced_action() {
        let value = TransactionDescr::Ordinary {
            credit_first: false,
            storage_ph: None,
            credit_ph: Some(credit_phase()),
            compute_ph: compute_vm(),
            action: Some(action_phase()),
            aborted: true,
            bounce: Some(TrBouncePhase::Ok {
                msg_size: StorageUsed::new(BigUint::from(1u8), BigUint::from(1u8)),
                msg_fees: Grams::from(2),
                fwd_fees: Grams::from(3),
            }),
            destroyed: true,
        };

        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn storage_only_description_roundtrips() {
        let value = TransactionDescr::Storage {
            storage_ph: storage_phase(),
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn tick_tock_description_roundtrips() {
        let value = TransactionDescr::TickTock {
            is_tock: true,
            storage_ph: storage_phase(),
            compute_ph: compute_vm(),
            action: Some(action_phase()),
            aborted: false,
            destroyed: true,
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn split_prepare_description_roundtrips() {
        let value = TransactionDescr::SplitPrepare {
            split_info: split_info(),
            storage_ph: Some(storage_phase()),
            compute_ph: compute_skipped(),
            action: None,
            aborted: true,
            destroyed: false,
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn split_install_description_roundtrips_with_typed_prepare_transaction() {
        let prepare_transaction = Box::new(simple_transaction());
        let value = TransactionDescr::SplitInstall {
            split_info: split_info(),
            prepare_transaction: prepare_transaction.clone(),
            installed: true,
        };

        let decoded = roundtrip(&value);
        assert_eq!(decoded, value);
        match decoded {
            TransactionDescr::SplitInstall {
                prepare_transaction: decoded,
                ..
            } => assert_eq!(decoded, prepare_transaction),
            _ => panic!("expected split install"),
        }
    }

    #[test]
    fn merge_prepare_description_roundtrips() {
        let value = TransactionDescr::MergePrepare {
            split_info: split_info(),
            storage_ph: storage_phase(),
            aborted: true,
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn merge_install_description_roundtrips_with_typed_prepare_transaction() {
        let prepare_transaction = Box::new(simple_transaction());
        let value = TransactionDescr::MergeInstall {
            split_info: split_info(),
            prepare_transaction: prepare_transaction.clone(),
            storage_ph: None,
            credit_ph: Some(credit_phase()),
            compute_ph: compute_vm(),
            action: Some(action_phase()),
            aborted: false,
            destroyed: true,
        };

        let decoded = roundtrip(&value);
        assert_eq!(decoded, value);
        match decoded {
            TransactionDescr::MergeInstall {
                prepare_transaction: decoded,
                ..
            } => assert_eq!(decoded, prepare_transaction),
            _ => panic!("expected merge install"),
        }
    }

    #[test]
    fn unknown_and_truncated_transaction_description_tags_are_rejected() {
        let mut builder = Builder::new();
        store_tag(&mut builder, "1").unwrap();
        let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "TransactionDescr",
                actual_bits,
                ..
            } if actual_bits == "1"
        ));

        let mut builder = Builder::new();
        store_tag(&mut builder, "000").unwrap();
        let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "TransactionDescr",
                actual_bits,
                ..
            } if actual_bits == "000"
        ));
    }

    #[test]
    fn invalid_and_truncated_compute_skip_reason_tags_are_rejected() {
        let mut builder = Builder::new();
        store_tag(&mut builder, "111").unwrap();
        let err = ComputeSkipReason::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "ComputeSkipReason",
                actual_bits,
                ..
            } if actual_bits == "111"
        ));

        let mut builder = Builder::new();
        store_tag(&mut builder, "11").unwrap();
        let err = ComputeSkipReason::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::TagMismatch {
                constructor: "ComputeSkipReason",
                actual_bits,
                ..
            } if actual_bits == "11"
        ));
    }

    #[test]
    fn malformed_referenced_action_phase_payload_is_reported() {
        let mut invalid_action = Builder::new();
        invalid_action.store_bit(true).unwrap();

        let mut builder = Builder::new();
        store_tag(&mut builder, "0000").unwrap();
        builder.store_bit(false).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_bit(false).unwrap();
        compute_skipped().store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_ref(invalid_action.build().unwrap()).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_bit(false).unwrap();

        let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "TrActionPhase",
                ..
            }
        ));
    }

    #[test]
    fn compute_vm_malformed_referenced_payload_is_reported() {
        let mut builder = Builder::new();
        store_tag(&mut builder, "1").unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(false).unwrap();
        builder.store_bit(false).unwrap();
        Grams::from(1).store_tlb(&mut builder).unwrap();
        builder.store_ref(cell_with_bits(&[0x80], 1)).unwrap();

        let err = TrComputePhase::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                schema: "TrComputePhase.vm",
                ..
            }
        ));
    }

    #[test]
    fn exact_transaction_description_decode_rejects_trailing_data() {
        let value = TransactionDescr::Storage {
            storage_ph: storage_phase(),
        };
        let mut builder = Builder::new();
        value.store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
    }
}
