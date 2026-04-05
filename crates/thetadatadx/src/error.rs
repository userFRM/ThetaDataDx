// reason: Error types with thiserror derive don't benefit from pedantic lints.
#![allow(clippy::doc_markdown, clippy::uninlined_format_args)]
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
        // Extract http_status_code from gRPC metadata and enrich the error
        // message with the `ThetaData` error name when available.
        let metadata_str = format!("{:?}", s.metadata());
        if let Some(td_err) = tdbe::errors::error_from_grpc_metadata(&metadata_str) {
            let enriched = tonic::Status::new(
                s.code(),
                format!(
                    "{} (ThetaData: {} -- {})",
                    s.message(),
                    td_err.name,
                    td_err.description
                ),
            );
            Self::Status(Box::new(enriched))
        } else {
            Self::Status(Box::new(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_without_metadata_passes_through() {
        let status = tonic::Status::internal("something went wrong");
        let err = Error::from(status);
        let msg = format!("{}", err);
        assert!(msg.contains("something went wrong"));
    }
}
