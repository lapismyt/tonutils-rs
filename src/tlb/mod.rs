//! TL-B model layer.
//!
//! This module provides the minimal runtime surface for hand-written TL-B
//! codecs. It intentionally does not include derive macros, schema parsing, or
//! built-in blockchain models.

use crate::tvm::{Builder, Cell, Slice};
use num_bigint::BigUint;
use std::sync::Arc;
use thiserror::Error;

pub mod message;
pub mod transaction;

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
