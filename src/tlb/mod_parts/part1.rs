use crate::tvm::{Builder, Cell, HashmapE, Slice};
use num_bigint::BigUint;
use std::sync::Arc;
use thiserror::Error;

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
                builder.store_uint_custom::<$ty>(*self, BITS)?;
                Ok(())
            }
        }

        impl<const BITS: usize> LoadBits<BITS> for $ty {
            fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
                Ok(slice.load_uint_custom::<$ty>(BITS)?)
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
        builder.store_uint_custom::<u128>(*self, BITS)?;
        Ok(())
    }
}

impl<const BITS: usize> LoadBits<BITS> for u128 {
    fn load_bits_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(slice.load_uint_custom::<u128>(BITS)?)
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
    let byte_len = slice.load_uint_custom::<u64>(length_bits)? as usize;
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
