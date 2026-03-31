use thiserror::Error;

/// Encoding-layer errors. No networking, no async -- only codec and protocol failures.
#[derive(Error, Debug)]
pub enum Error {
    #[error("FPSS protocol error: {0}")]
    FpssProtocol(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
