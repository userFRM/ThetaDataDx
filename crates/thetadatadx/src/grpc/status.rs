//! gRPC status, parsed from HTTP/2 trailers.
//!
//! Real implementation lands in the next commit. This stub exists so the
//! [`crate::grpc`] module can compile with `Codec` fully tested in
//! isolation.

/// Placeholder gRPC status type. Filled in by the trailer-parser commit.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Status {
    /// Numeric gRPC status code (`grpc-status` trailer).
    pub code: u32,
    /// Human-readable status message (`grpc-message` trailer; may be empty).
    pub message: String,
}
