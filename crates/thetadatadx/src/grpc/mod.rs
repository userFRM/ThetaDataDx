//! MDDS gRPC transport over the reference Rust gRPC stack (tonic).
//!
//! # Shape
//!
//! The MDDS code path is server-streaming gRPC over HTTP/2 + TLS with
//! prost-encoded protobuf payloads. This module is a thin wrapper over
//! `tonic::transport::Channel` + `tonic::client::Grpc`:
//!
//! - [`Channel`] — one gRPC channel (one HTTP/2 connection, reconnected
//!   in place by the underlying stack when it dies). Owns the connect
//!   parameters (host, port, optional rustls config, per-frame decode
//!   ceiling) and dispatches server-streaming RPCs.
//! - [`ServerStreaming`] — async [`futures_core::Stream`] adapter over
//!   `tonic::Streaming` that yields decoded `Resp` values and maps every
//!   error into the crate's own [`ChannelError`] taxonomy at the module
//!   boundary.
//! - [`Status`] — the crate's own gRPC status type (numeric code,
//!   message, optional `google.rpc.RetryInfo` backoff hint). Built from
//!   `tonic::Status`; no third-party status type crosses this module's
//!   boundary.
//! - [`ChannelPool`] — least-loaded fan-out across `N` channels so a
//!   workload spreads across distinct HTTP/2 connections (one
//!   connection-level flow-control window each) instead of contending
//!   on one.
//! - [`endpoints`] — a hand-written example RPC plus bench helpers.
//!
//! Per-chunk decode (zstd decompress + prost `DataTable` decode) runs
//! inline on the request task — measured faster than handing chunks to
//! a dedicated decoder pool at every production-reachable concurrency
//! (see `docs/architecture/in-house-grpc-transport.md`).
//!
//! # Surface hygiene
//!
//! No `tonic` type appears in any public signature. The crate's public
//! error surface is [`crate::Error`]; this module's [`ChannelError`] /
//! [`Status`] are crate-internal (re-opened only under the private
//! `__test-helpers` feature) and convert into `crate::Error` at the
//! crate boundary via `From` impls in [`crate::error`].
//!
//! # Error classification
//!
//! - A genuine server status (carried in `grpc-status` trailers or a
//!   trailers-only response) surfaces as [`ChannelError::Rpc`].
//! - Connection-level transport death (GOAWAY, IO failure, connect
//!   failure on the in-place reconnect path) surfaces as
//!   [`ChannelError::ConnectionClosed`]; the retry shell in
//!   `crate::mdds::macros` classifies it as transient and re-dispatches,
//!   by which point the underlying stack has lazily reconnected.
//! - Per-stream `RST_STREAM` (any reason code) surfaces as
//!   [`ChannelError::H2Stream`]; the connection itself is healthy and
//!   the next RPC on the same channel can succeed.
//! - A per-call deadline surfaces as [`ChannelError::DeadlineExceeded`]
//!   whether it fires during the open phase or mid-stream.

// Sub-modules carry transport infrastructure consumed only by the
// crate itself and by `__test-helpers`-gated integration tests + benches.
// They are reachable as `thetadatadx::grpc::*` only when that private
// feature (or `cfg(test)`) is active — the parent `pub(crate) mod grpc`
// guard in `lib.rs` is what keeps them out of the shipped rlib. The
// `pub` visibility here scopes within the (then-private) module tree.
pub mod channel;
// `endpoints` is a hand-written `stock_list_symbols` example plus
// `bench_support` helpers used exclusively by the gRPC benches and
// the `grpc_stock_list_symbols` integration test. Production RPCs go
// through the macro-generated `crate::mdds::*` endpoints directly.
// Gating on `__test-helpers` keeps the example out of the default rlib.
#[cfg(feature = "__test-helpers")]
pub mod endpoints;
pub mod pool;
pub mod status;
pub mod stream;

// Production-path re-exports — used by `crate::mdds` and `crate::error`.
// These names are reachable as `thetadatadx::grpc::*` only when the
// `__test-helpers` private feature is enabled (see the `pub(crate) mod
// grpc` guard in `lib.rs`); without the feature they are crate-internal
// only.
pub use channel::{Channel, ChannelError, ChannelTuning};
pub use pool::{ChannelLease, ChannelPool};
pub use status::Status;
pub use stream::ServerStreaming;

// Test-only re-exports — only reachable when the `__test-helpers` feature
// is enabled.
#[cfg(feature = "__test-helpers")]
pub use endpoints::stock_list_symbols;
