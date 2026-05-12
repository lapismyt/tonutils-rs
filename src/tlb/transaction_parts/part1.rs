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
        builder.store_uint_custom::<u8>(self.split_depth as u8, 5)?;
        self.balance.store_tlb(builder)?;
        Ok(())
    }
}

impl TlbDeserialize for DepthBalanceInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let split_depth = slice.load_uint_custom::<u8>(5)? as u8;
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
        builder.store_uint::<u8>(0x72)?;
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
        builder.store_uint_custom::<u16>(self.outmsg_cnt, OUT_MSG_KEY_BITS)?;
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
        let outmsg_cnt = slice.load_uint_custom::<u16>(OUT_MSG_KEY_BITS)?;
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

