//! Internal unsigned integer support for fixed-width cell APIs.

use anyhow::{Result, bail};
use num_bigint::BigUint;

mod private {
    pub trait Sealed {}

    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for u32 {}
    impl Sealed for u64 {}
    impl Sealed for u128 {}
}

/// Sealed unsigned primitive accepted by fixed-width cell integer helpers.
///
/// This trait is public only because it appears in public method bounds. It is
/// sealed, so downstream crates cannot implement it for additional types.
#[doc(hidden)]
pub trait UnsignedInteger: private::Sealed + Copy + Sized {
    /// Natural bit width of the primitive.
    const BITS: usize;

    /// Converts the primitive into a big unsigned integer.
    fn to_big_uint(self) -> BigUint;

    /// Converts a big unsigned integer into the primitive.
    fn from_big_uint(value: BigUint) -> Result<Self>;
}

macro_rules! impl_small_uint {
    ($ty:ty, $bits:expr) => {
        impl UnsignedInteger for $ty {
            const BITS: usize = $bits;

            fn to_big_uint(self) -> BigUint {
                BigUint::from(self)
            }

            fn from_big_uint(value: BigUint) -> Result<Self> {
                let digits = value.to_u64_digits();
                let value = digits.first().copied().unwrap_or(0);
                if digits.len() > 1 || value > <$ty>::MAX as u64 {
                    bail!("Loaded unsigned integer does not fit {}", stringify!($ty));
                }
                Ok(value as $ty)
            }
        }
    };
}

impl_small_uint!(u8, 8);
impl_small_uint!(u16, 16);
impl_small_uint!(u32, 32);
impl_small_uint!(u64, 64);

impl UnsignedInteger for u128 {
    const BITS: usize = 128;

    fn to_big_uint(self) -> BigUint {
        BigUint::from(self)
    }

    fn from_big_uint(value: BigUint) -> Result<Self> {
        let digits = value.to_u64_digits();
        if digits.len() > 2 {
            bail!("Loaded unsigned integer does not fit u128");
        }
        let low = digits.first().copied().unwrap_or(0) as u128;
        let high = digits.get(1).copied().unwrap_or(0) as u128;
        Ok(low | (high << 64))
    }
}
