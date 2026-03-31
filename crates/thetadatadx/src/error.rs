use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("gRPC transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("gRPC status: {0}")]
    Status(Box<tonic::Status>),

    #[error("Decompression failed: {0}")]
    Decompress(String),

    #[error("Protobuf decode failed: {0}")]
    Decode(String),

    #[error("No data returned")]
    NoData,

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("FPSS connection error: {0}")]
    Fpss(String),

    #[error("FPSS protocol error: {0}")]
    FpssProtocol(String),

    #[error("FPSS disconnected: {0}")]
    FpssDisconnected(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),
}

impl From<tonic::Status> for Error {
    fn from(s: tonic::Status) -> Self {
        Self::Status(Box::new(s))
    }
}

impl From<tdx_encoding::Error> for Error {
    fn from(e: tdx_encoding::Error) -> Self {
        match e {
            tdx_encoding::Error::FpssProtocol(msg) => Self::FpssProtocol(msg),
            tdx_encoding::Error::Io(io_err) => Self::Io(io_err),
        }
    }
}
