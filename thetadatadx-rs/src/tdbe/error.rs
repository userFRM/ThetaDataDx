//! Encoding-layer errors for `ThetaData` Binary Encoding. Bridged into the
//! crate's public [`crate::Error`] through a `From` impl so callers can use
//! `?` across the offline analytics surface.

use thiserror::Error;

/// Encoding-layer errors for `ThetaData` Binary Encoding.
#[derive(Error, Debug)]
pub enum Error {
    /// FIT nibble decoding failure (malformed input, unexpected terminator).
    #[error("FIT decode error: {0}")]
    Decode(String),

    /// Configuration / input validation error in the encoding layer.
    #[error("Configuration error: {0}")]
    Config(String),

    /// I/O error during read/write operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
