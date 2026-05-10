//! TL-B model layer.
//!
//! This module provides runtime traits for hand-written TL-B codecs, built-in
//! Phase 1 blockchain models, and a deterministic schema parser/check-summary
//! workflow in [`schema`]. It intentionally does not include a proc-macro derive
//! crate in Phase 1; schema-driven checks and hand-written codecs share the
//! same [`TlbSerialize`] and [`TlbDeserialize`] traits.

use crate::tvm::{Builder, Cell, HashmapE, Slice};
use num_bigint::BigUint;
use std::sync::Arc;
use thiserror::Error;

pub mod block;
pub mod message;
pub mod schema;
pub mod transaction;

#[cfg(feature = "tlb-derive")]
pub use tonutils_tlb_derive::{Tlb, Tlb as TlbDerive};

pub use block::{
    Block, BlockExtra, BlockIdExtTlb, BlockInfo, BlockPrevInfo, ConfigParams, ExtBlkRef,
    HashUpdate, McBlockExtra, MerkleProof, MerkleUpdate, ShardIdent, ShardState, ShardStateUnsplit,
    ValueFlow,
};
pub use message::{
    AccStatusChange, Anycast, CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Grams,
    LibRef, Message, MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, OutAction, OutList,
    SimpleLib, StateInit, StateInitWithLibs, StorageUsed, TickTock, TrActionPhase,
};
pub use transaction::{
    Account, AccountBlock, AccountState, AccountStatus, AccountStorage, ComputeSkipReason,
    DepthBalanceInfo, HashUpdateAccount, ShardAccount, ShardAccountBlocks, ShardAccounts,
    SplitMergeInfo, StorageExtraInfo, StorageInfo, TrBouncePhase, TrComputePhase, TrCreditPhase,
    TrStoragePhase, Transaction, TransactionDescr,
};

/// Result type used by TL-B codecs.
pub type Result<T> = std::result::Result<T, TlbError>;

/// Errors returned by TL-B model codecs.
#[derive(Debug, Error)]
pub enum TlbError {
    /// Low-level TVM cell, builder, or slice operation failed.
    #[error(transparent)]
    Tvm(#[from] anyhow::Error),

    /// A constructor tag did not match the expected schema tag.
    #[error(
        "TL-B constructor tag mismatch for {constructor}: expected {expected_bits}, got {actual_bits}"
    )]
    TagMismatch {
        /// Constructor or result name being decoded.
        constructor: &'static str,
        /// Expected tag bits.
        expected_bits: &'static str,
        /// Actual bits read before the mismatch or underflow.
        actual_bits: String,
    },

    /// Exact decoding left unread bits or references in the slice.
    #[error("TL-B trailing data after exact decode: {bits} bits and {refs} refs remaining")]
    TrailingData {
        /// Unread bit count.
        bits: usize,
        /// Unread reference count.
        refs: usize,
    },

    /// A value was valid at the bit level but not canonical for its schema.
    #[error("TL-B non-canonical {schema}: {reason}")]
    NonCanonicalValue {
        /// Schema value name.
        schema: &'static str,
        /// Canonicality failure details.
        reason: String,
    },

    /// A referenced child cell failed to decode as the expected schema value.
    #[error("invalid TL-B reference payload for {schema}")]
    InvalidReferencePayload {
        /// Referenced schema value name.
        schema: &'static str,
        /// Child decode failure.
        #[source]
        source: Box<TlbError>,
    },

    /// Schema-specific validation failed.
    #[error("TL-B schema error in {schema}: {message}")]
    CustomSchema {
        /// Schema value name.
        schema: &'static str,
        /// Validation message.
        message: String,
    },
}

/// Trait for values that can be serialized into TL-B cell data.
pub trait TlbSerialize {
    /// Stores this value into the provided cell builder.
    fn store_tlb(&self, builder: &mut Builder) -> Result<()>;

    /// Serializes this value into a standalone cell.
    fn to_cell(&self) -> Result<Arc<Cell>> {
        let mut builder = Builder::new();
        self.store_tlb(&mut builder)?;
        Ok(builder.build()?)
    }
}

/// Trait for values that can be deserialized from TL-B cell data.
pub trait TlbDeserialize: Sized {
    /// Loads this value from the current slice position.
    fn load_tlb(slice: &mut Slice) -> Result<Self>;

    /// Deserializes this value from a standalone cell and requires exact
    /// consumption of all bits and references.
    fn from_cell(cell: Arc<Cell>) -> Result<Self> {
        let mut slice = Slice::new(cell);
        let value = Self::load_tlb(&mut slice)?;
        ensure_empty(&slice)?;
        Ok(value)
    }
}

/// Descriptive metadata for a TL-B schema constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlbScheme {
    /// Constructor name from the schema.
    pub constructor: &'static str,
    /// Result type name from the schema.
    pub result: &'static str,
    /// Static constructor tag bits, when the constructor has a fixed tag.
    pub tag_bits: Option<&'static str>,
}

/// Runtime representation for `Either L R`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Either<L, R> {
    /// Left branch, encoded with branch bit `0`.
    Left(L),
    /// Right branch, encoded with branch bit `1`.
    Right(R),
}

/// Referenced TL-B value helper for `^T` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRef<T>(pub T);

/// Raw cell payload helper for schema positions that intentionally preserve a
/// cell instead of decoding it into a typed model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCell(pub Arc<Cell>);

/// Canonical `VarUInteger` wrapper parameterized by prefix width in bits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarUInteger<const LEN_BITS: usize>(pub BigUint);

/// Typed `HashmapE n X` wrapper that uses `TlbSerialize` and `TlbDeserialize`
/// for values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlbHashmapE<T, const KEY_BITS: usize>(pub HashmapE<T>);

/// Stores a fixed-width primitive value used by `#[derive(Tlb)]`.
pub trait StoreBits<const BITS: usize> {
    /// Stores `self` in exactly `BITS` bits.
    fn store_bits_tlb(&self, builder: &mut Builder) -> Result<()>;
}

/// Loads a fixed-width primitive value used by `#[derive(Tlb)]`.
pub trait LoadBits<const BITS: usize>: Sized {
    /// Loads `Self` from exactly `BITS` bits.
    fn load_bits_tlb(slice: &mut Slice) -> Result<Self>;
}

/// Stores a fixed constructor tag described as a string of `0` and `1` bits.
pub fn store_tag(builder: &mut Builder, tag_bits: &'static str) -> Result<()> {
    for bit in tag_bits.bytes() {
        match bit {
            b'0' => {
                builder.store_bit(false)?;
            }
            b'1' => {
                builder.store_bit(true)?;
            }
            _ => {
                return Err(TlbError::CustomSchema {
                    schema: "constructor tag",
                    message: format!("invalid tag bit byte {bit}"),
                });
            }
        }
    }
    Ok(())
}

/// Checks and consumes a fixed constructor tag from a slice.
pub fn expect_tag(
    slice: &mut Slice,
    constructor: &'static str,
    expected_bits: &'static str,
) -> Result<()> {
    let mut actual = String::with_capacity(expected_bits.len());
    for expected in expected_bits.bytes() {
        if expected != b'0' && expected != b'1' {
            return Err(TlbError::CustomSchema {
                schema: constructor,
                message: format!("invalid expected tag bit byte {expected}"),
            });
        }

        let bit = match slice.load_bit() {
            Ok(bit) => bit,
            Err(_) => {
                return Err(TlbError::TagMismatch {
                    constructor,
                    expected_bits,
                    actual_bits: actual,
                });
            }
        };
        actual.push(if bit { '1' } else { '0' });

        let expected_bool = expected == b'1';
        if bit != expected_bool {
            return Err(TlbError::TagMismatch {
                constructor,
                expected_bits,
                actual_bits: actual,
            });
        }
    }
    Ok(())
}

/// Requires that a slice has no remaining bits or references.
pub fn ensure_empty(slice: &Slice) -> Result<()> {
    if slice.is_empty() {
        Ok(())
    } else {
        Err(TlbError::TrailingData {
            bits: slice.remaining_bits(),
            refs: slice.remaining_refs(),
        })
    }
}

/// Stores `Maybe T` as a one-bit presence marker followed by the value.
pub fn store_maybe<T: TlbSerialize>(builder: &mut Builder, value: &Option<T>) -> Result<()> {
    match value {
        Some(value) => {
            builder.store_bit(true)?;
            value.store_tlb(builder)
        }
        None => {
            builder.store_bit(false)?;
            Ok(())
        }
    }
}

/// Loads `Maybe T` from a one-bit presence marker.
pub fn load_maybe<T: TlbDeserialize>(slice: &mut Slice) -> Result<Option<T>> {
    if slice.load_bit()? {
        Ok(Some(T::load_tlb(slice)?))
    } else {
        Ok(None)
    }
}

/// Stores `Either L R` as a one-bit branch marker followed by the selected value.
pub fn store_either<L, R>(builder: &mut Builder, value: &Either<L, R>) -> Result<()>
where
    L: TlbSerialize,
    R: TlbSerialize,
{
    match value {
        Either::Left(left) => {
            builder.store_bit(false)?;
            left.store_tlb(builder)
        }
        Either::Right(right) => {
            builder.store_bit(true)?;
            right.store_tlb(builder)
        }
    }
}

/// Loads `Either L R` from a one-bit branch marker.
pub fn load_either<L, R>(slice: &mut Slice) -> Result<Either<L, R>>
where
    L: TlbDeserialize,
    R: TlbDeserialize,
{
    if slice.load_bit()? {
        Ok(Either::Right(R::load_tlb(slice)?))
    } else {
        Ok(Either::Left(L::load_tlb(slice)?))
    }
}

impl<T: TlbSerialize> TlbSerialize for CellRef<T> {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_ref_tlb(builder, &self.0)
    }
}

impl<T: TlbDeserialize> TlbDeserialize for CellRef<T> {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self(load_ref_tlb(slice, "CellRef")?))
    }
}

impl<T: TlbSerialize> TlbSerialize for Option<T> {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_maybe(builder, self)
    }
}

impl<T: TlbDeserialize> TlbDeserialize for Option<T> {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        load_maybe(slice)
    }
}

impl<L, R> TlbSerialize for Either<L, R>
where
    L: TlbSerialize,
    R: TlbSerialize,
{
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_either(builder, self)
    }
}

impl<L, R> TlbDeserialize for Either<L, R>
where
    L: TlbDeserialize,
    R: TlbDeserialize,
{
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        load_either(slice)
    }
}

impl TlbSerialize for RawCell {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(&self.0)?;
        Ok(())
    }
}

impl TlbDeserialize for RawCell {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let mut builder = Builder::new();
        let remaining_bits = slice.remaining_bits();
        if remaining_bits > 0 {
            let bits = slice.load_bits(remaining_bits)?;
            builder.store_bits(&bits, remaining_bits)?;
        }
        for reference in slice.load_remaining_refs()? {
            builder.store_ref(reference)?;
        }
        Ok(Self(builder.build()?))
    }
}

impl TlbSerialize for Arc<Cell> {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(self)?;
        Ok(())
    }
}

impl TlbDeserialize for Arc<Cell> {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(RawCell::load_tlb(slice)?.0)
    }
}

impl TlbSerialize for bool {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_bit(*self)?;
        Ok(())
    }
}

impl TlbDeserialize for bool {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(slice.load_bit()?)
    }
}

macro_rules! impl_uint_tlb {
    ($ty:ty) => {
        impl<const BITS: usize> StoreBits<BITS> for $ty {
            fn store_bits_tlb(&self, builder: &mut Builder) -> Result<()> {
                builder.store_uint(*self as u64, BITS)?;
                Ok(())
            }
        }

        impl<const BITS: usize> LoadBits<BITS> for $ty {
            fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
                let value = slice.load_uint(BITS)?;
                <$ty>::try_from(value).map_err(|_| TlbError::CustomSchema {
                    schema: stringify!($ty),
                    message: format!("decoded value {value} does not fit {}", stringify!($ty)),
                })
            }
        }
    };
}

macro_rules! impl_int_tlb {
    ($ty:ty) => {
        impl<const BITS: usize> StoreBits<BITS> for $ty {
            fn store_bits_tlb(&self, builder: &mut Builder) -> Result<()> {
                builder.store_int(*self as i64, BITS)?;
                Ok(())
            }
        }

        impl<const BITS: usize> LoadBits<BITS> for $ty {
            fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
                let value = slice.load_int(BITS)?;
                <$ty>::try_from(value).map_err(|_| TlbError::CustomSchema {
                    schema: stringify!($ty),
                    message: format!("decoded value {value} does not fit {}", stringify!($ty)),
                })
            }
        }
    };
}

impl_uint_tlb!(u8);
impl_uint_tlb!(u16);
impl_uint_tlb!(u32);
impl_uint_tlb!(u64);

impl<const BITS: usize> StoreBits<BITS> for u128 {
    fn store_bits_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_big_uint(&BigUint::from(*self), BITS)?;
        Ok(())
    }
}

impl<const BITS: usize> LoadBits<BITS> for u128 {
    fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
        let value = slice.load_big_uint(BITS)?;
        let digits = value.to_u64_digits();
        if digits.len() > 2 {
            return Err(TlbError::CustomSchema {
                schema: "u128",
                message: format!("decoded value {value} does not fit u128"),
            });
        }
        let low = digits.first().copied().unwrap_or(0) as u128;
        let high = digits.get(1).copied().unwrap_or(0) as u128;
        Ok(low | (high << 64))
    }
}

impl_int_tlb!(i8);
impl_int_tlb!(i16);
impl_int_tlb!(i32);
impl_int_tlb!(i64);

impl<const BITS: usize> StoreBits<BITS> for i128 {
    fn store_bits_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_big_int(&num_bigint::BigInt::from(*self), BITS)?;
        Ok(())
    }
}

impl<const BITS: usize> LoadBits<BITS> for i128 {
    fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
        let value = slice.load_big_int(BITS)?;
        i128::try_from(value.clone()).map_err(|_| TlbError::CustomSchema {
            schema: "i128",
            message: format!("decoded value {value} does not fit i128"),
        })
    }
}

impl StoreBits<256> for [u8; 32] {
    fn store_bits_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_bytes(self)?;
        Ok(())
    }
}

impl LoadBits<256> for [u8; 32] {
    fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
        let mut bytes = [0; 32];
        bytes.copy_from_slice(&slice.load_bytes(32)?);
        Ok(bytes)
    }
}

impl<const LEN_BITS: usize> TlbSerialize for VarUInteger<LEN_BITS> {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_var_uint(builder, &self.0, LEN_BITS)
    }
}

impl<const LEN_BITS: usize> TlbDeserialize for VarUInteger<LEN_BITS> {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self(load_var_uint(slice, LEN_BITS)?))
    }
}

impl<T, const KEY_BITS: usize> TlbSerialize for TlbHashmapE<T, KEY_BITS>
where
    T: TlbSerialize,
{
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_hashmap_e_with(&self.0, |builder, value| {
            value.store_tlb(builder).map_err(anyhow::Error::from)
        })?;
        Ok(())
    }
}

impl<T, const KEY_BITS: usize> TlbDeserialize for TlbHashmapE<T, KEY_BITS>
where
    T: TlbDeserialize,
{
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self(slice.load_hashmap_e_with(KEY_BITS, |slice| {
            T::load_tlb(slice).map_err(anyhow::Error::from)
        })?))
    }
}

/// Stores a referenced `^T` value in a child cell.
pub fn store_ref_tlb<T: TlbSerialize>(builder: &mut Builder, value: &T) -> Result<()> {
    builder.store_ref(value.to_cell()?)?;
    Ok(())
}

/// Loads a referenced `^T` value and requires exact child-cell consumption.
pub fn load_ref_tlb<T: TlbDeserialize>(slice: &mut Slice, schema: &'static str) -> Result<T> {
    let cell = slice.load_reference()?;
    T::from_cell(cell).map_err(|source| TlbError::InvalidReferencePayload {
        schema,
        source: Box::new(source),
    })
}

/// Stores a canonical `VarUInteger` using an existing TVM builder helper.
pub fn store_var_uint(builder: &mut Builder, value: &BigUint, length_bits: usize) -> Result<()> {
    builder.store_var_big_uint(value, length_bits)?;
    Ok(())
}

/// Loads a canonical `VarUInteger`.
///
/// The `length_bits` argument is the width of the byte-length prefix. For
/// `VarUInteger 16`, pass `4`.
pub fn load_var_uint(slice: &mut Slice, length_bits: usize) -> Result<BigUint> {
    let byte_len = slice.load_uint(length_bits)? as usize;
    let max_len = max_var_uint_bytes(length_bits)?;
    if byte_len > max_len {
        return Err(TlbError::NonCanonicalValue {
            schema: "VarUInteger",
            reason: format!("byte length {byte_len} exceeds maximum {max_len}"),
        });
    }

    if byte_len == 0 {
        return Ok(BigUint::from(0u8));
    }

    let bytes = slice.load_bytes(byte_len)?;
    if bytes.first() == Some(&0) {
        return Err(TlbError::NonCanonicalValue {
            schema: "VarUInteger",
            reason: "non-zero values must use the shortest byte length".to_string(),
        });
    }

    Ok(BigUint::from_bytes_be(&bytes))
}

fn max_var_uint_bytes(length_bits: usize) -> Result<usize> {
    if length_bits >= usize::BITS as usize {
        return Err(TlbError::CustomSchema {
            schema: "VarUInteger",
            message: format!("length prefix width {length_bits} does not fit usize"),
        });
    }
    Ok((1usize << length_bits) - 1)
}

#[cfg(test)]
mod offline_fixture_tests {
    use super::*;
    use crate::tvm::{
        Address, BitKey, Builder, Cell, HashmapAug, HashmapAugE, HashmapAugLeaf, HashmapE, Slice,
        base64_to_boc, boc_to_hex, hex_to_boc,
    };
    use num_bigint::BigUint;
    use std::fmt::Debug;
    use std::sync::Arc;

    struct TlbFixture {
        name: &'static str,
        source: &'static str,
        encoded: FixtureEncoding,
        expected_root_hash: &'static str,
        decoded_type: &'static str,
    }

    enum FixtureEncoding {
        Hex(&'static str),
        Base64(&'static str),
    }

    fn assert_fixture<T>(fixture: &TlbFixture, expected: &T)
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + Debug,
    {
        assert!(!fixture.name.is_empty());
        assert!(!fixture.source.is_empty());
        assert!(!fixture.decoded_type.is_empty());

        let cell = fixture_cell(fixture);
        assert_eq!(hex::encode(cell.hash()), fixture.expected_root_hash);

        let decoded = T::from_cell(cell.clone()).unwrap();
        assert_eq!(&decoded, expected, "{}", fixture.name);

        let canonical_cell = decoded.to_cell().unwrap();
        assert_eq!(canonical_cell.hash(), cell.hash(), "{}", fixture.name);
        assert_eq!(
            boc_to_hex(&canonical_cell, false).unwrap(),
            boc_to_hex(&cell, false).unwrap(),
            "{}",
            fixture.name
        );
    }

    fn assert_trailing_data_is_rejected<T>(fixture: &TlbFixture)
    where
        T: TlbDeserialize + Debug,
    {
        let cell = fixture_cell(fixture);
        let mut builder = Builder::new();
        builder.store_bits(cell.data(), cell.bit_len()).unwrap();
        for reference in cell.references() {
            builder.store_ref(reference.clone()).unwrap();
        }
        builder.store_bit(true).unwrap();

        let err = T::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(
            matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }),
            "{}",
            fixture.name
        );
    }

    fn fixture_cell(fixture: &TlbFixture) -> Arc<Cell> {
        match fixture.encoded {
            FixtureEncoding::Hex(hex) => hex_to_boc(hex).unwrap(),
            FixtureEncoding::Base64(base64) => base64_to_boc(base64).unwrap(),
        }
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn std_address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn account_address() -> MsgAddressInt {
        MsgAddressInt::std(std_address(0x11))
    }

    fn message_fixture_value() -> Message {
        Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: account_address(),
                import_fee: Grams::from(1),
            },
            init: Some(Either::Right(StateInit {
                code: Some(cell_with_bits(&[0xA5], 8)),
                ..StateInit::empty()
            })),
            body: Either::Right(cell_with_bits(&[0x80], 1)),
        }
    }

    fn relaxed_message_fixture_value() -> MessageRelaxed {
        MessageRelaxed {
            info: CommonMsgInfoRelaxed::Internal {
                ihr_disabled: true,
                bounce: false,
                bounced: false,
                src: MsgAddress::Ext(MsgAddressExt::None),
                dest: MsgAddressInt::std(std_address(0x22)),
                value: CurrencyCollection::grams(Grams::from(7)),
                extra_flags: BigUint::from(2u8),
                fwd_fee: Grams::from(3),
                created_lt: 4,
                created_at: 5,
            },
            init: None,
            body: Either::Right(cell_with_bits(&[0xAD, 0x80], 9)),
        }
    }

    fn currency_collection_fixture_value() -> CurrencyCollection {
        let mut other = HashmapE::new(32);
        other
            .insert_bit_key(BitKey::from_u64(7, 32).unwrap(), BigUint::from(42u8))
            .unwrap();
        CurrencyCollection {
            grams: Grams::from(123),
            other,
        }
    }

    fn state_init_fixture_value() -> StateInit {
        StateInit {
            fixed_prefix_length: Some(5),
            special: Some(TickTock {
                tick: true,
                tock: false,
            }),
            code: Some(cell_with_bits(&[0xAA], 8)),
            data: Some(cell_with_bits(&[0xBC], 6)),
            library: None,
        }
    }

    fn storage_phase() -> TrStoragePhase {
        TrStoragePhase {
            storage_fees_collected: Grams::from(7),
            storage_fees_due: Some(Grams::from(8)),
            status_change: AccStatusChange::Frozen,
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

    fn account_fixture_value() -> Account {
        Account::Full {
            addr: account_address(),
            storage_stat: storage_info(),
            storage: account_storage(),
        }
    }

    fn transaction_descr_fixture_value() -> TransactionDescr {
        TransactionDescr::Storage {
            storage_ph: storage_phase(),
        }
    }

    fn transaction_fixture_value() -> Transaction {
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
            out_msgs: HashmapE::new(15),
            total_fees: CurrencyCollection::grams(Grams::from(3)),
            state_update: hash_update(),
            description: transaction_descr_fixture_value(),
        }
    }

    fn depth_balance(split_depth: u8, grams: u64) -> DepthBalanceInfo {
        DepthBalanceInfo {
            split_depth,
            balance: CurrencyCollection::grams(Grams::from(grams)),
        }
    }

    fn shard_accounts_fixture_value() -> ShardAccounts {
        let shard_account = ShardAccount {
            account: account_fixture_value(),
            last_trans_hash: [0x44; 32],
            last_trans_lt: 12,
        };
        let root = HashmapAug::from_entries(
            256,
            vec![HashmapAugLeaf {
                key: BitKey::from_bits(vec![0x11; 32], 256).unwrap(),
                value: shard_account,
                extra: depth_balance(7, 100),
            }],
            depth_balance(7, 100),
        )
        .unwrap();
        ShardAccounts {
            accounts: HashmapAugE::with_root(256, root, depth_balance(7, 100)).unwrap(),
        }
    }

    fn account_block_fixture_value() -> AccountBlock {
        let root = HashmapAug::from_entries(
            64,
            vec![HashmapAugLeaf {
                key: BitKey::from_u64(7, 64).unwrap(),
                value: transaction_fixture_value(),
                extra: CurrencyCollection::grams(Grams::from(8)),
            }],
            CurrencyCollection::grams(Grams::from(8)),
        )
        .unwrap();
        AccountBlock {
            account_addr: [0x55; 32],
            transactions: root,
            state_update: hash_update(),
        }
    }

    fn shard_account_blocks_fixture_value() -> ShardAccountBlocks {
        let root = HashmapAug::from_entries(
            256,
            vec![HashmapAugLeaf {
                key: BitKey::from_bits(vec![0x22; 32], 256).unwrap(),
                value: account_block_fixture_value(),
                extra: CurrencyCollection::grams(Grams::from(8)),
            }],
            CurrencyCollection::grams(Grams::from(8)),
        )
        .unwrap();
        ShardAccountBlocks {
            blocks: HashmapAugE::with_root(256, root, CurrencyCollection::grams(Grams::from(8)))
                .unwrap(),
        }
    }

    #[test]
    fn message_account_and_transaction_offline_fixtures_roundtrip() {
        const SOURCE: &str = "synthetic schema-derived offline fixture from implemented TL-B model";

        let message = TlbFixture {
            name: "message-with-referenced-state-init-and-body",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c72010104010032030002a5010124000001c0024788002222222222222222222222222222222222222222222222222222222222222222203e0102",
            ),
            expected_root_hash: "ae43183ebb6674776cf91aa612ed19a6a6f5ab5199ee87eaf36ec62e4ff323e3",
            decoded_type: "Message Any",
        };
        assert_fixture(&message, &message_fixture_value());
        assert_trailing_data_is_rejected::<Message>(&message);

        let relaxed_message = TlbFixture {
            name: "relaxed-message-with-referenced-body",
            source: SOURCE,
            encoded: FixtureEncoding::Base64(
                "te6ccgEBAgEAOgEAA63AAWZCABERERERERERERERERERERERERERERERERERERERERERCDhAhAwAAAAAAAAAEAAAABUA",
            ),
            expected_root_hash: "fdd15c56139da31cacbc9ab2e7435726d89303e6be82cc259e515e3dff8f548c",
            decoded_type: "MessageRelaxed Any",
        };
        assert_fixture(&relaxed_message, &relaxed_message_fixture_value());
        assert_trailing_data_is_rejected::<MessageRelaxed>(&relaxed_message);

        let state_init = TlbFixture {
            name: "state-init-with-prefix-special-code-data",
            source: SOURCE,
            encoded: FixtureEncoding::Hex("b5ee9c7201010301000c020002aa0001be020397680001"),
            expected_root_hash: "33ef718f8d73687800d7c90a5202b4f12703fdd38cdfbf0486a8be78828fe51b",
            decoded_type: "StateInit",
        };
        assert_fixture(&state_init, &state_init_fixture_value());

        let currency = TlbFixture {
            name: "currency-collection-with-extra-currency",
            source: SOURCE,
            encoded: FixtureEncoding::Hex("b5ee9c7201010201000e01000da0000000070954010317bc00"),
            expected_root_hash: "467a118b2a65ebfd9e10fbfd1aa44af31e356145073abb735f12d5b1b07df88c",
            decoded_type: "CurrencyCollection",
        };
        assert_fixture(&currency, &currency_collection_fixture_value());

        let transaction = TlbFixture {
            name: "storage-only-transaction",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101040100aa03000120008272aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb00071107884503b1710101010101010101010101010101010101010101010101010101010101010100000000000000007202020202020202020202020202020202020202020202020202020202020202000000000000000066553f100000142068000102",
            ),
            expected_root_hash: "cdc9b5675d0c34623ccab0b8c28d58aadcfc137634833cee1ea35ba127ecd916",
            decoded_type: "Transaction",
        };
        assert_fixture(&transaction, &transaction_fixture_value());
        assert_trailing_data_is_rejected::<Transaction>(&transaction);

        let transaction_descr = TlbFixture {
            name: "storage-transaction-description",
            source: SOURCE,
            encoded: FixtureEncoding::Base64("te6ccgEBAQEABgAABxEHiEU="),
            expected_root_hash: "c69351ef516847eb1e5bd23f96058ea43510a14f626b189ffdd910ba1e99fbdd",
            decoded_type: "TransactionDescr",
        };
        assert_fixture(&transaction_descr, &transaction_descr_fixture_value());

        let account = TlbFixture {
            name: "full-active-account",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101010100570000a9c001111111111111111111111111111111111111111111111111111111111111111204600e66666666666666666666666666666666666666666666666666666666666666632a9f880c410000000000000002c59104",
            ),
            expected_root_hash: "c11a6be3cd1afdb472d4cc26c62dce6fa6bb9ebd2c0660f12025c03e00dc2595",
            decoded_type: "Account",
        };
        assert_fixture(&account, &account_fixture_value());
    }

    #[test]
    fn augmented_account_collection_offline_fixtures_roundtrip() {
        const SOURCE: &str = "synthetic schema-derived offline fixture from implemented TL-B model";

        let shard_accounts = TlbFixture {
            name: "shard-accounts-single-entry",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101030100ac0200a9c001111111111111111111111111111111111111111111111111111111111111111204600e66666666666666666666666666666666666666666666666666666666666666632a9f880c410000000000000002c591040197a00222222222222222222222222222222222222222222222222222222222222222271642222222222222222222222222222222222222222222222222222222222222222000000000000000640001059c591001",
            ),
            expected_root_hash: "21f2fe5665f466ab2e9d52c223da658af42d45beba9e0664df21b9b873361fa3",
            decoded_type: "ShardAccounts",
        };
        assert_fixture(&shard_accounts, &shard_accounts_fixture_value());

        let account_block = TlbFixture {
            name: "account-block-single-transaction",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101050100da04000120008272aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb00071107884503b1710101010101010101010101010101010101010101010101010101010101010100000000000000007202020202020202020202020202020202020202020202020202020202020202000000000000000066553f100000142068000102025755555555555555555555555555555555555555555555555555555555555555555a00000000000000003884200301",
            ),
            expected_root_hash: "397ab2e4d9d18889064c7dc44db29470a30588595e1dc6b93ddfa5822821442b",
            decoded_type: "AccountBlock",
        };
        assert_fixture(&account_block, &account_block_fixture_value());

        let shard_account_blocks = TlbFixture {
            name: "shard-account-blocks-single-entry",
            source: SOURCE,
            encoded: FixtureEncoding::Base64(
                "te6ccgECBgEAAQIFAAEgAIJyqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7uwAHEQeIRQOxcQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEAAAAAAAAAAHICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAAAAAAAAAABmVT8QAAAUIGgAAQICnaAEREREREREREREREREREREREREREREREREREREREREREIQVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVaAAAAAAAAAAA4hCADAQEDiEIE",
            ),
            expected_root_hash: "b1ebb413e70b1bde953a55e8e8ac51899ac25adb28f3e02aac97ad5b2fa9ace7",
            decoded_type: "ShardAccountBlocks",
        };
        assert_fixture(&shard_account_blocks, &shard_account_blocks_fixture_value());
    }

    #[test]
    fn hashmap_e_fixture_preserves_canonical_root_reference_and_labels() {
        let fixture = TlbFixture {
            name: "hashmap-e-two-entry-labels",
            source: "synthetic schema-derived offline fixture for HashmapE 4 uint8",
            encoded: FixtureEncoding::Hex(
                "b5ee9c72010104010011030003d00c0003d01402014800010101c002",
            ),
            expected_root_hash: "9c02490e70a529c7242d63c3e85f273d8080b37c64abf8ebc2bcbf8713dc6db9",
            decoded_type: "HashmapE 4 uint8",
        };
        let cell = fixture_cell(&fixture);
        assert_eq!(hex::encode(cell.hash()), fixture.expected_root_hash);
        assert_eq!(cell.reference_count(), 1);

        let mut slice = Slice::new(cell.clone());
        assert!(slice.load_bit().unwrap());
        let decoded = Slice::new(cell)
            .load_hashmap_e_with(4, |slice| slice.load_uint(8))
            .unwrap();
        let entries: Vec<_> = decoded
            .iter()
            .map(|(key, value)| (key.to_u64().unwrap(), *value))
            .collect();
        assert_eq!(entries, vec![(0, 1), (4, 2)]);
    }

    #[test]
    fn hashmap_aug_e_fixtures_preserve_top_and_fork_extras() {
        let empty: HashmapAugE<u64, u64> = HashmapAugE::empty(4, 88);
        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_e_with(
                &empty,
                |builder, value| {
                    builder.store_uint(*value, 8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint(*extra, 8)?;
                    Ok(())
                },
            )
            .unwrap();
        let cell = builder.build().unwrap();
        assert_eq!(cell.reference_count(), 0);
        let mut slice = Slice::new(cell);
        let decoded_empty = slice
            .load_hashmap_aug_e_with(4, |slice| slice.load_uint(8), |slice| slice.load_uint(8))
            .unwrap();
        assert!(decoded_empty.is_empty());
        assert_eq!(*decoded_empty.extra(), 88);
        ensure_empty(&slice).unwrap();

        let fixture = TlbFixture {
            name: "hashmap-aug-e-three-entry-extras",
            source: "synthetic schema-derived offline fixture for HashmapAugE 4 uint8 uint8",
            encoded: FixtureEncoding::Hex(
                "b5ee9c72010106010020050005d0500c0005d0a0140203136000010005b83c070203136002030103ac4004",
            ),
            expected_root_hash: "6ad8187666c7eef33e1fa3281cc4e18fb0bb9793c11388864bd319c84a1d0612",
            decoded_type: "HashmapAugE 4 uint8 uint8",
        };
        let cell = fixture_cell(&fixture);
        assert_eq!(hex::encode(cell.hash()), fixture.expected_root_hash);
        assert_eq!(cell.reference_count(), 1);

        let mut slice = Slice::new(cell);
        let decoded = slice
            .load_hashmap_aug_e_with(4, |slice| slice.load_uint(8), |slice| slice.load_uint(8))
            .unwrap();
        let root = decoded.root().unwrap();
        let leaves: Vec<_> = root
            .iter()
            .map(|(key, value, extra)| (key.to_u64().unwrap(), *value, *extra))
            .collect();
        assert_eq!(leaves, vec![(0, 1, 10), (4, 2, 20), (12, 3, 30)]);
        assert_eq!(*decoded.extra(), 88);
        assert!(root.fork_extras().iter().all(|fork| fork.extra == 77));
        ensure_empty(&slice).unwrap();
    }
}

#[cfg(test)]
mod phase1_checked_fixture_tests {
    use super::*;
    use crate::tvm::{Address, Builder, Cell, HashmapE, boc_to_hex, hex_to_boc};
    use num_bigint::BigUint;
    use serde::Deserialize;
    use std::fmt::Debug;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct FixtureSet {
        schema_revision: String,
        fixtures: Vec<Fixture>,
    }

    #[derive(Debug, Deserialize)]
    struct Fixture {
        name: String,
        source: String,
        capture_date: String,
        upstream_commit_or_endpoint: String,
        decoded_type: String,
        root_hash: String,
        canonical_reserialization: String,
        boc_hex: String,
    }

    fn fixture_set(json: &str) -> FixtureSet {
        let set: FixtureSet = serde_json::from_str(json).unwrap();
        assert!(!set.schema_revision.is_empty());
        assert!(!set.fixtures.is_empty());
        set
    }

    fn assert_fixture<T>(fixture: &Fixture, expected_type: &str, expected_value: T)
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + Debug,
    {
        assert!(!fixture.name.is_empty());
        assert!(!fixture.source.is_empty());
        assert!(!fixture.capture_date.is_empty());
        assert!(!fixture.upstream_commit_or_endpoint.is_empty());
        assert_eq!(fixture.decoded_type, expected_type);
        assert!(
            fixture
                .canonical_reserialization
                .contains("canonical BoC without index table or CRC32")
        );

        let cell = hex_to_boc(&fixture.boc_hex).unwrap();
        assert_eq!(
            hex::encode(cell.hash()),
            fixture.root_hash,
            "{}",
            fixture.name
        );

        let decoded = T::from_cell(cell.clone()).unwrap();
        assert_eq!(decoded, expected_value, "{}", fixture.name);
        assert_eq!(
            boc_to_hex(&decoded.to_cell().unwrap(), false).unwrap(),
            fixture.boc_hex,
            "{}",
            fixture.name
        );
    }

    fn find<'a>(set: &'a FixtureSet, name: &str) -> &'a Fixture {
        set.fixtures
            .iter()
            .find(|fixture| fixture.name == name)
            .unwrap_or_else(|| panic!("missing fixture {name}"))
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn std_address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn account_address() -> MsgAddressInt {
        MsgAddressInt::std(std_address(0x11))
    }

    fn message_fixture_value() -> Message {
        Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: account_address(),
                import_fee: Grams::from(1),
            },
            init: Some(Either::Right(StateInit {
                code: Some(cell_with_bits(&[0xA5], 8)),
                ..StateInit::empty()
            })),
            body: Either::Right(cell_with_bits(&[0x80], 1)),
        }
    }

    fn relaxed_message_fixture_value() -> MessageRelaxed {
        MessageRelaxed {
            info: CommonMsgInfoRelaxed::Internal {
                ihr_disabled: true,
                bounce: false,
                bounced: false,
                src: MsgAddress::Ext(MsgAddressExt::None),
                dest: MsgAddressInt::std(std_address(0x22)),
                value: CurrencyCollection::grams(Grams::from(7)),
                extra_flags: BigUint::from(2u8),
                fwd_fee: Grams::from(3),
                created_lt: 4,
                created_at: 5,
            },
            init: None,
            body: Either::Right(cell_with_bits(&[0xAD, 0x80], 9)),
        }
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

    fn account_fixture_value() -> Account {
        Account::Full {
            addr: account_address(),
            storage_stat: storage_info(),
            storage: account_storage(),
        }
    }

    fn transaction_fixture_value() -> Transaction {
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
            out_msgs: HashmapE::new(15),
            total_fees: CurrencyCollection::grams(Grams::from(3)),
            state_update: hash_update(),
            description: TransactionDescr::Storage {
                storage_ph: storage_phase(),
            },
        }
    }

    fn simple_transaction() -> Transaction {
        transaction_fixture_value()
    }

    #[test]
    fn phase1_account_message_transaction_fixtures_are_checked() {
        let set = fixture_set(include_str!(
            "../../fixtures/phase1/account_message_transaction.json"
        ));
        assert_fixture(
            find(&set, "message-with-referenced-state-init-and-body"),
            "Message Any",
            message_fixture_value(),
        );
        assert_fixture(
            find(&set, "relaxed-message-with-referenced-body"),
            "MessageRelaxed Any",
            relaxed_message_fixture_value(),
        );
        assert_fixture(
            find(&set, "storage-only-transaction"),
            "Transaction",
            transaction_fixture_value(),
        );
        assert_fixture(
            find(&set, "full-active-account"),
            "Account",
            account_fixture_value(),
        );
    }

    #[test]
    fn phase1_transaction_description_fixtures_cover_all_exit_cases() {
        let set = fixture_set(include_str!(
            "../../fixtures/phase1/transaction_descriptions.json"
        ));
        assert_fixture(
            find(&set, "ordinary-transaction-description"),
            "TransactionDescr::Ordinary",
            TransactionDescr::Ordinary {
                credit_first: true,
                storage_ph: Some(storage_phase()),
                credit_ph: Some(credit_phase()),
                compute_ph: compute_skipped(),
                action: None,
                aborted: false,
                bounce: Some(TrBouncePhase::NegativeFunds),
                destroyed: false,
            },
        );
        assert_fixture(
            find(&set, "tick-tock-transaction-description"),
            "TransactionDescr::TickTock",
            TransactionDescr::TickTock {
                is_tock: true,
                storage_ph: storage_phase(),
                compute_ph: compute_vm(),
                action: Some(action_phase()),
                aborted: false,
                destroyed: true,
            },
        );
        assert_fixture(
            find(&set, "split-prepare-transaction-description"),
            "TransactionDescr::SplitPrepare",
            TransactionDescr::SplitPrepare {
                split_info: split_info(),
                storage_ph: Some(storage_phase()),
                compute_ph: compute_skipped(),
                action: None,
                aborted: true,
                destroyed: false,
            },
        );
        assert_fixture(
            find(&set, "split-install-transaction-description"),
            "TransactionDescr::SplitInstall",
            TransactionDescr::SplitInstall {
                split_info: split_info(),
                prepare_transaction: Box::new(simple_transaction()),
                installed: true,
            },
        );
        assert_fixture(
            find(&set, "merge-prepare-transaction-description"),
            "TransactionDescr::MergePrepare",
            TransactionDescr::MergePrepare {
                split_info: split_info(),
                storage_ph: storage_phase(),
                aborted: true,
            },
        );
        assert_fixture(
            find(&set, "merge-install-transaction-description"),
            "TransactionDescr::MergeInstall",
            TransactionDescr::MergeInstall {
                split_info: split_info(),
                prepare_transaction: Box::new(simple_transaction()),
                storage_ph: None,
                credit_ph: Some(credit_phase()),
                compute_ph: compute_vm(),
                action: Some(action_phase()),
                aborted: false,
                destroyed: true,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Tiny {
        value: u8,
    }

    impl TlbSerialize for Tiny {
        fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
            store_tag(builder, "101")?;
            builder.store_uint(self.value as u64, 5)?;
            Ok(())
        }
    }

    impl TlbDeserialize for Tiny {
        fn load_tlb(slice: &mut Slice) -> Result<Self> {
            expect_tag(slice, "tiny$101", "101")?;
            let value = slice.load_uint(5)? as u8;
            Ok(Self { value })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct BitValue(bool);

    impl TlbSerialize for BitValue {
        fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
            builder.store_bit(self.0)?;
            Ok(())
        }
    }

    impl TlbDeserialize for BitValue {
        fn load_tlb(slice: &mut Slice) -> Result<Self> {
            Ok(Self(slice.load_bit()?))
        }
    }

    #[test]
    fn trait_roundtrip_for_hand_written_type() {
        let original = Tiny { value: 0b1_0110 };
        let cell = original.to_cell().unwrap();
        let decoded = Tiny::from_cell(cell).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn constructor_tag_success_and_mismatch() {
        let mut builder = Builder::new();
        store_tag(&mut builder, "10").unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        expect_tag(&mut slice, "ok$10", "10").unwrap();
        ensure_empty(&slice).unwrap();

        let mut builder = Builder::new();
        store_tag(&mut builder, "11").unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        let err = expect_tag(&mut slice, "bad$10", "10").unwrap_err();
        assert!(matches!(err, TlbError::TagMismatch { .. }));
    }

    #[test]
    fn exact_decode_rejects_trailing_bits_and_refs() {
        let mut builder = Builder::new();
        Tiny { value: 3 }.store_tlb(&mut builder).unwrap();
        builder.store_bit(true).unwrap();
        let err = Tiny::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));

        let child = Tiny { value: 1 }.to_cell().unwrap();
        let mut builder = Builder::new();
        Tiny { value: 3 }.store_tlb(&mut builder).unwrap();
        builder.store_ref(child).unwrap();
        let err = Tiny::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TrailingData { bits: 0, refs: 1 }));
    }

    #[test]
    fn maybe_present_and_absent_paths() {
        let mut builder = Builder::new();
        store_maybe(&mut builder, &Some(BitValue(true))).unwrap();
        store_maybe::<BitValue>(&mut builder, &None).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(
            load_maybe::<BitValue>(&mut slice).unwrap(),
            Some(BitValue(true))
        );
        assert_eq!(load_maybe::<BitValue>(&mut slice).unwrap(), None);
        ensure_empty(&slice).unwrap();
    }

    #[test]
    fn either_left_and_right_paths() {
        let mut builder = Builder::new();
        store_either::<BitValue, Tiny>(&mut builder, &Either::Left(BitValue(false))).unwrap();
        store_either::<BitValue, Tiny>(&mut builder, &Either::Right(Tiny { value: 7 })).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(
            load_either::<BitValue, Tiny>(&mut slice).unwrap(),
            Either::Left(BitValue(false))
        );
        assert_eq!(
            load_either::<BitValue, Tiny>(&mut slice).unwrap(),
            Either::Right(Tiny { value: 7 })
        );
        ensure_empty(&slice).unwrap();
    }

    #[test]
    fn referenced_value_requires_child_slice_consumption() {
        let mut builder = Builder::new();
        store_ref_tlb(&mut builder, &Tiny { value: 9 }).unwrap();
        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(
            load_ref_tlb::<Tiny>(&mut slice, "tiny_ref").unwrap(),
            Tiny { value: 9 }
        );
        ensure_empty(&slice).unwrap();

        let mut child = Builder::new();
        Tiny { value: 9 }.store_tlb(&mut child).unwrap();
        child.store_bit(false).unwrap();
        let mut parent = Builder::new();
        parent.store_ref(child.build().unwrap()).unwrap();
        let mut slice = Slice::new(parent.build().unwrap());
        let err = load_ref_tlb::<Tiny>(&mut slice, "tiny_ref").unwrap_err();
        assert!(matches!(
            err,
            TlbError::InvalidReferencePayload {
                source,
                ..
            } if matches!(*source, TlbError::TrailingData { bits: 1, refs: 0 })
        ));
    }

    #[test]
    fn var_uint_accepts_canonical_zero_and_non_zero() {
        let mut builder = Builder::new();
        store_var_uint(&mut builder, &BigUint::from(0u8), 4).unwrap();
        store_var_uint(&mut builder, &BigUint::from(0x1234u16), 4).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        assert_eq!(load_var_uint(&mut slice, 4).unwrap(), BigUint::from(0u8));
        assert_eq!(
            load_var_uint(&mut slice, 4).unwrap(),
            BigUint::from(0x1234u16)
        );
        ensure_empty(&slice).unwrap();
    }

    #[test]
    fn var_uint_rejects_overlong_non_canonical_encoding() {
        let mut builder = Builder::new();
        builder.store_uint(2, 4).unwrap();
        builder.store_bytes(&[0, 1]).unwrap();

        let mut slice = Slice::new(builder.build().unwrap());
        let err = load_var_uint(&mut slice, 4).unwrap_err();
        assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
    }
}
