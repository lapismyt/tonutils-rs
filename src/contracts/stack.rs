use std::sync::Arc;

use num_bigint::{BigInt, BigUint, Sign};
use thiserror::Error;

use crate::tlb::{MsgAddress, MsgAddressInt, TlbDeserialize, TlbSerialize};
use crate::tvm::{Address, Cell, Slice, TvmStack, TvmStackEntry};

/// Error returned when converting between Rust values and get-method stack data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TvmStackConversionError {
    #[error("TVM stack arity mismatch: expected {expected}, got {actual}")]
    StackArityMismatch { expected: usize, actual: usize },
    #[error("TVM stack entry type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("TVM stack integer {value} is outside {target}")]
    IntegerOutOfRange { target: &'static str, value: String },
    #[error("TVM bool stack value must be -1 or 0, got {value}")]
    InvalidBool { value: String },
    #[error("TVM address stack value is malformed: {reason}")]
    MalformedAddress { reason: String },
}

/// Converts a Rust value into a full get-method argument stack.
pub trait ToTvmStack {
    fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError>;
}

/// Converts a full get-method result stack into a Rust value.
pub trait FromTvmStack: Sized {
    fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError>;
}

/// Converts a Rust value into one TVM stack entry.
pub trait ToTvmStackEntry {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError>;
}

/// Converts one TVM stack entry into a Rust value.
pub trait FromTvmStackEntry: Sized {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError>;
}

impl ToTvmStack for () {
    fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError> {
        Ok(TvmStack::empty())
    }
}

impl FromTvmStack for () {
    fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError> {
        expect_arity(stack.entries().len(), 0)?;
        Ok(())
    }
}

impl ToTvmStack for TvmStack {
    fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError> {
        Ok(self)
    }
}

impl FromTvmStack for TvmStack {
    fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError> {
        Ok(stack)
    }
}

impl ToTvmStack for Vec<TvmStackEntry> {
    fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError> {
        Ok(TvmStack::new(self))
    }
}

impl FromTvmStack for Vec<TvmStackEntry> {
    fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError> {
        Ok(stack.entries().to_vec())
    }
}

impl ToTvmStackEntry for TvmStackEntry {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        Ok(self)
    }
}

impl FromTvmStackEntry for TvmStackEntry {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        Ok(entry)
    }
}

impl ToTvmStackEntry for BigInt {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        Ok(TvmStackEntry::Int(self))
    }
}

impl FromTvmStackEntry for BigInt {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        match entry {
            TvmStackEntry::Int(value) => Ok(value),
            other => Err(type_mismatch("integer", &other)),
        }
    }
}

impl ToTvmStackEntry for BigUint {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        Ok(TvmStackEntry::Int(BigInt::from(self)))
    }
}

impl FromTvmStackEntry for BigUint {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        let value = BigInt::from_tvm_stack_entry(entry)?;
        value
            .to_biguint()
            .ok_or_else(|| integer_out_of_range("BigUint", value))
    }
}

macro_rules! impl_single_stack {
    ($ty:ty) => {
        impl ToTvmStack for $ty {
            fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError> {
                Ok(TvmStack::new(vec![self.to_tvm_stack_entry()?]))
            }
        }

        impl FromTvmStack for $ty {
            fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError> {
                let entries = stack.entries().to_vec();
                expect_arity(entries.len(), 1)?;
                Self::from_tvm_stack_entry(entries.into_iter().next().unwrap())
            }
        }
    };
}

impl_single_stack!(TvmStackEntry);
impl_single_stack!(BigInt);
impl_single_stack!(BigUint);

impl ToTvmStackEntry for bool {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        Ok(TvmStackEntry::Int(if self {
            BigInt::from(-1)
        } else {
            BigInt::from(0)
        }))
    }
}

impl FromTvmStackEntry for bool {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        let value = BigInt::from_tvm_stack_entry(entry)?;
        if value == BigInt::from(-1) {
            Ok(true)
        } else if value == BigInt::from(0) {
            Ok(false)
        } else {
            Err(TvmStackConversionError::InvalidBool {
                value: value.to_string(),
            })
        }
    }
}

impl_single_stack!(bool);

impl ToTvmStackEntry for Address {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        MsgAddressInt::std(self)
            .to_cell()
            .map(TvmStackEntry::Slice)
            .map_err(|source| TvmStackConversionError::MalformedAddress {
                reason: source.to_string(),
            })
    }
}

impl FromTvmStackEntry for Address {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        let TvmStackEntry::Slice(cell) = entry else {
            return Err(type_mismatch("address slice", &entry));
        };
        let mut slice = Slice::new(cell);
        let address = MsgAddress::load_tlb(&mut slice).map_err(|source| {
            TvmStackConversionError::MalformedAddress {
                reason: source.to_string(),
            }
        })?;
        if !slice.is_empty() {
            return Err(TvmStackConversionError::MalformedAddress {
                reason: "address slice has trailing data".to_string(),
            });
        }
        match address {
            MsgAddress::Int(MsgAddressInt::Std {
                anycast: None,
                address,
            }) => Ok(address),
            MsgAddress::Int(MsgAddressInt::Std {
                anycast: Some(_), ..
            }) => Err(TvmStackConversionError::MalformedAddress {
                reason: "standard address contains anycast".to_string(),
            }),
            MsgAddress::Int(MsgAddressInt::Var { .. }) => {
                Err(TvmStackConversionError::MalformedAddress {
                    reason: "variable-length internal addresses are unsupported".to_string(),
                })
            }
            MsgAddress::Ext(_) => Err(TvmStackConversionError::MalformedAddress {
                reason: "external addresses are unsupported".to_string(),
            }),
        }
    }
}

impl_single_stack!(Address);

impl ToTvmStackEntry for Arc<Cell> {
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        Ok(TvmStackEntry::Cell(self))
    }
}

impl FromTvmStackEntry for Arc<Cell> {
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        match entry {
            TvmStackEntry::Cell(cell) => Ok(cell),
            other => Err(type_mismatch("cell", &other)),
        }
    }
}

impl_single_stack!(Arc<Cell>);

impl<T> ToTvmStackEntry for Option<T>
where
    T: ToTvmStackEntry,
{
    fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
        self.map(ToTvmStackEntry::to_tvm_stack_entry)
            .transpose()
            .map(|entry| entry.unwrap_or(TvmStackEntry::Null))
    }
}

impl<T> FromTvmStackEntry for Option<T>
where
    T: FromTvmStackEntry,
{
    fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
        match entry {
            TvmStackEntry::Null => Ok(None),
            other => T::from_tvm_stack_entry(other).map(Some),
        }
    }
}

impl<T> ToTvmStack for Option<T>
where
    T: ToTvmStackEntry,
{
    fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError> {
        Ok(TvmStack::new(vec![self.to_tvm_stack_entry()?]))
    }
}

impl<T> FromTvmStack for Option<T>
where
    T: FromTvmStackEntry,
{
    fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError> {
        let entries = stack.entries().to_vec();
        expect_arity(entries.len(), 1)?;
        Self::from_tvm_stack_entry(entries.into_iter().next().unwrap())
    }
}

macro_rules! impl_signed_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ToTvmStackEntry for $ty {
                fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
                    Ok(TvmStackEntry::Int(BigInt::from(self)))
                }
            }

            impl FromTvmStackEntry for $ty {
                fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
                    let value = BigInt::from_tvm_stack_entry(entry)?;
                    <$ty>::try_from(&value).map_err(|_| integer_out_of_range(stringify!($ty), value))
                }
            }

            impl_single_stack!($ty);
        )*
    };
}

macro_rules! impl_unsigned_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ToTvmStackEntry for $ty {
                fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
                    Ok(TvmStackEntry::Int(BigInt::from(self)))
                }
            }

            impl FromTvmStackEntry for $ty {
                fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
                    let value = BigInt::from_tvm_stack_entry(entry)?;
                    if value.sign() == Sign::Minus {
                        return Err(integer_out_of_range(stringify!($ty), value));
                    }
                    <$ty>::try_from(&value).map_err(|_| integer_out_of_range(stringify!($ty), value))
                }
            }

            impl_single_stack!($ty);
        )*
    };
}

impl_signed_int!(i8, i16, i32, i64, i128, isize);
impl_unsigned_int!(u8, u16, u32, u64, u128, usize);

macro_rules! impl_tuple_stack {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> ToTvmStack for ($($name,)+)
        where
            $($name: ToTvmStackEntry),+
        {
            #[allow(non_snake_case)]
            fn to_tvm_stack(self) -> Result<TvmStack, TvmStackConversionError> {
                let ($($name,)+) = self;
                Ok(TvmStack::new(vec![$($name.to_tvm_stack_entry()?),+]))
            }
        }

        impl<$($name),+> FromTvmStack for ($($name,)+)
        where
            $($name: FromTvmStackEntry),+
        {
            fn from_tvm_stack(stack: TvmStack) -> Result<Self, TvmStackConversionError> {
                let mut entries = stack.entries().to_vec().into_iter();
                let expected = count_idents!($($name),+);
                expect_arity(entries.len(), expected)?;
                Ok(($($name::from_tvm_stack_entry(entries.next().unwrap())?,)+))
            }
        }

        impl<$($name),+> ToTvmStackEntry for ($($name,)+)
        where
            $($name: ToTvmStackEntry),+
        {
            #[allow(non_snake_case)]
            fn to_tvm_stack_entry(self) -> Result<TvmStackEntry, TvmStackConversionError> {
                let ($($name,)+) = self;
                Ok(TvmStackEntry::Tuple(vec![$($name.to_tvm_stack_entry()?),+]))
            }
        }

        impl<$($name),+> FromTvmStackEntry for ($($name,)+)
        where
            $($name: FromTvmStackEntry),+
        {
            fn from_tvm_stack_entry(entry: TvmStackEntry) -> Result<Self, TvmStackConversionError> {
                let TvmStackEntry::Tuple(entries) = entry else {
                    return Err(type_mismatch("tuple", &entry));
                };
                let mut entries = entries.into_iter();
                let expected = count_idents!($($name),+);
                expect_arity(entries.len(), expected)?;
                Ok(($($name::from_tvm_stack_entry(entries.next().unwrap())?,)+))
            }
        }
    };
}

macro_rules! count_idents {
    ($($name:ident),+ $(,)?) => {
        <[()]>::len(&[$(replace_expr!(($name) ())),+])
    };
}

macro_rules! replace_expr {
    ($_t:tt $sub:expr) => {
        $sub
    };
}

impl_tuple_stack!(A);
impl_tuple_stack!(A, B);
impl_tuple_stack!(A, B, C);
impl_tuple_stack!(A, B, C, D);
impl_tuple_stack!(A, B, C, D, E);
impl_tuple_stack!(A, B, C, D, E, F);
impl_tuple_stack!(A, B, C, D, E, F, G);
impl_tuple_stack!(A, B, C, D, E, F, G, H);

fn expect_arity(actual: usize, expected: usize) -> Result<(), TvmStackConversionError> {
    if actual == expected {
        Ok(())
    } else {
        Err(TvmStackConversionError::StackArityMismatch { expected, actual })
    }
}

fn type_mismatch(expected: &'static str, entry: &TvmStackEntry) -> TvmStackConversionError {
    TvmStackConversionError::TypeMismatch {
        expected,
        actual: stack_entry_name(entry),
    }
}

fn integer_out_of_range(target: &'static str, value: BigInt) -> TvmStackConversionError {
    TvmStackConversionError::IntegerOutOfRange {
        target,
        value: value.to_string(),
    }
}

fn stack_entry_name(entry: &TvmStackEntry) -> &'static str {
    match entry {
        TvmStackEntry::Null => "null",
        TvmStackEntry::Int(_) => "integer",
        TvmStackEntry::Cell(_) => "cell",
        TvmStackEntry::Slice(_) => "slice",
        TvmStackEntry::Tuple(_) => "tuple",
        TvmStackEntry::List(_) => "list",
        TvmStackEntry::Unsupported(_) => "unsupported",
    }
}
