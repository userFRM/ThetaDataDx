//! Encoding-layer errors for `ThetaData` Binary Encoding.

use thiserror::Error;

/// Encoding-layer errors for `ThetaData` Binary Encoding.
#[derive(Error, Debug)]
pub enum Error {
    /// FIT nibble decoding failure (malformed input, unexpected terminator).
    #[error("FIT decode error: {0}")]
    Decode(String),

    /// FIE nibble encoding failure (invalid character).
    #[error("FIE encode error: {0}")]
    Encode(String),

    /// Value conversion error (e.g., enum from invalid discriminant).
    #[error("conversion error: {0}")]
    Conversion(String),

    /// I/O error during read/write operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience Result type.
pub type Result<T> = std::result::Result<T, Error>;
