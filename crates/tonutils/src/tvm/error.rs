use thiserror::Error;

/// Errors returned by TVM cell, builder, slice, dictionary, address, stack,
/// and Bag-of-Cells operations.
#[derive(Debug, Error)]
pub enum TvmError {
    /// Cell structure or exotic-cell validation failed.
    #[error("cell error: {0}")]
    Cell(String),
    /// Slice bounds or payload validation failed.
    #[error("slice error: {0}")]
    Slice(String),
    /// Builder capacity or value validation failed.
    #[error("builder error: {0}")]
    Builder(String),
    /// Dictionary key, label, or tree validation failed.
    #[error("dictionary error: {0}")]
    Dictionary(String),
    /// Bag-of-Cells serialization or parsing failed.
    #[error("BoC error: {0}")]
    Boc(String),
    /// TON address parsing or validation failed.
    #[error("address error: {0}")]
    Address(String),
    /// TVM stack serialization, decoding, or capacity validation failed.
    #[error("stack error: {0}")]
    Stack(String),
    /// Unsigned integer width or range validation failed.
    #[error("integer error: {0}")]
    Integer(String),
    /// Hexadecimal decoding failed.
    #[error(transparent)]
    Hex(#[from] hex::FromHexError),
    /// Base64 decoding failed.
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    /// A higher-layer callback failed while traversing dictionary or cell data.
    ///
    /// The wrapped error is kept intact so callers that own its concrete type
    /// can recover its structured details instead of parsing a rendered string.
    #[error(transparent)]
    Callback(Box<dyn std::error::Error + Send + Sync + 'static>),
    /// A lower-level operation failed without a more specific category.
    #[error("{0}")]
    Other(String),
}

/// Result type used by TVM operations.
pub type TvmResult<T> = Result<T, TvmError>;

macro_rules! cell_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Cell(format!($($arg)*)))
    };
}
pub(crate) use cell_bail;

macro_rules! slice_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Slice(format!($($arg)*)))
    };
}
pub(crate) use slice_bail;

macro_rules! builder_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Builder(format!($($arg)*)))
    };
}
pub(crate) use builder_bail;

macro_rules! dictionary_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Dictionary(format!($($arg)*)))
    };
}
pub(crate) use dictionary_bail;

macro_rules! boc_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Boc(format!($($arg)*)))
    };
}
pub(crate) use boc_bail;

macro_rules! address_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Address(format!($($arg)*)))
    };
}
pub(crate) use address_bail;

macro_rules! stack_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Stack(format!($($arg)*)))
    };
}
pub(crate) use stack_bail;

macro_rules! integer_bail {
    ($($arg:tt)*) => {
        return Err($crate::tvm::TvmError::Integer(format!($($arg)*)))
    };
}
pub(crate) use integer_bail;
