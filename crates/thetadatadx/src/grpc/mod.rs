//! In-house gRPC client built directly on the [`h2`] crate.
//!
//! # Why this exists
//!
//! The MDDS code path is server-streaming gRPC over HTTP/2 + TLS with
//! prost-encoded protobuf payloads. This module implements that path
//! directly: encode prost → frame → send DATA → poll response stream →
//! decode frames → parse trailers. No tower stack, no boxed bodies,
//! no `async-trait` dyn dispatch.
//!
//! # Wire shape
//!
//! gRPC over HTTP/2 (see <https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md>):
//!
//! ```text
//!   Request   :method  = POST
//!             :scheme  = https
//!             :path    = /<package>.<service>/<Method>
//!             :authority = <host>:<port>
//!             content-type = application/grpc+proto
//!             te = trailers
//!             user-agent = thetadatadx-grpc/<version>
//!
//!   Body      one or more length-prefix frames, each:
//!             [1 byte compressed flag] [4 bytes big-endian length] [payload]
//!
//!   Response  HTTP status MUST be 200 (failures travel via grpc-status,
//!             not HTTP status).
//!             Body: zero or more length-prefix frames.
//!             Trailers: grpc-status (required, numeric),
//!                       grpc-message (optional, UTF-8).
//! ```
//!
//! The compressed flag is always `0` on send; receive-side rejects `1`
//! and any reserved-bits byte. gRPC status codes follow
//! <https://grpc.github.io/grpc/core/md_doc_statuscodes.html>.
//!
//! # Hardening
//!
//! - Per-call deadlines via
//!   [`Channel::server_streaming_with_deadline`] cover both the open
//!   phase and the streaming phase.
//! - h2 errors are classified by scope before they reach the caller:
//!   - [`ChannelError::ConnectionClosed`] — connection-level death.
//!     Covers `GOAWAY` in either direction, IO failure on the h2
//!     transport, peer shutdown, and any failure observed during
//!     the open phase ([`Channel::server_streaming_with_deadline`]
//!     routes `ready()` / `send_request()` / `send_data()` errors
//!     through this classifier too — a connection that dies while
//!     we are admitting a stream is connection-level, not
//!     stream-level). The pool reacts by recycling the channel.
//!   - [`ChannelError::H2Stream`] — per-stream death. Covers
//!     `RST_STREAM` from the peer on the specific stream (any
//!     reason code: `CANCEL`, `REFUSED_STREAM`, `INTERNAL_ERROR`,
//!     etc.) plus h2 library-level protocol errors that affect only
//!     this stream. The connection itself is still healthy; the
//!     next RPC on the same channel can succeed.
//! - Dropping a [`ServerStreaming`] cancels the underlying h2 stream
//!   cleanly (sends `RST_STREAM`).
//! - `grpc-encoding: identity` is the only accepted body encoding;
//!   zstd-compressed `compressed_data` payloads inside `ResponseData`
//!   are decompressed by the existing [`crate::decode`] pipeline.
//!
//! # Module layout
//!
//! - [`Codec`] — prost encode/decode + 5-byte length-prefix framing.
//! - [`Status`] — HTTP/2 trailers parser (`grpc-status` + `grpc-message`).
//! - [`Channel`] — one HTTP/2 connection, driven by a background tokio
//!   task that lives as long as the channel.
//! - [`ServerStreaming`] — async [`futures_core::Stream`] adapter over
//!   the h2 response body that yields decoded `Resp` values frame by
//!   frame and surfaces the trailing [`Status`] at end of stream.
//! - [`ChannelPool`] — round-robin fan-out across `N` connections to
//!   exceed the per-connection `MAX_CONCURRENT_STREAMS` limit.
//! - [`endpoints`] — typed RPC functions, one per generated stub.

// Sub-modules carry transport infrastructure consumed only by the
// crate itself and by `__test-helpers`-gated integration tests + benches.
// They are reachable as `thetadatadx::grpc::*` only when that private
// feature (or `cfg(test)`) is active — the parent `pub(crate) mod grpc`
// guard in `lib.rs` is what keeps them out of the shipped rlib. The
// `pub` visibility here scopes within the (then-private) module tree.
pub mod channel;
pub mod codec;
pub mod decoder_pool;
// `endpoints` is a hand-written `stock_list_symbols` example plus
// `bench_support` helpers used exclusively by the gRPC benches and
// the `grpc_stock_list_symbols` integration test. Production RPCs go
// through the macro-generated `crate::mdds::*` endpoints directly.
// Gating on `__test-helpers` keeps the example out of the default rlib.
#[cfg(feature = "__test-helpers")]
pub mod endpoints;
pub mod pool;
pub mod stage_pipeline;
pub mod status;
pub mod stream;

// Production-path re-exports — used by `crate::mdds`, `crate::error`,
// and the `flatfiles` session bootstrap. These names are reachable as
// `thetadatadx::grpc::*` only when the `__test-helpers` private feature
// is enabled (see the `pub(crate) mod grpc` guard in `lib.rs`); without
// the feature they are crate-internal only.
pub use channel::{Channel, ChannelError};
pub use decoder_pool::{default_decoder_thread_count, DecoderHandle, DecoderPool};
pub use pool::{ChannelLease, ChannelPool};
pub use status::Status;
pub use stream::ServerStreaming;

// Test-only re-exports — only reachable when the `__test-helpers` feature
// is enabled. The underlying items are themselves cfg-gated; gating the
// re-exports avoids `unused import` errors in the default-features build.
#[cfg(feature = "__test-helpers")]
pub use codec::{Codec, CodecError};
#[cfg(feature = "__test-helpers")]
pub use decoder_pool::{DecodeResult, DecoderPoolError, DecoderSubmitError, DecoderWaitStrategy};
#[cfg(feature = "__test-helpers")]
pub use endpoints::stock_list_symbols;
#[cfg(feature = "__test-helpers")]
pub use stage_pipeline::{DecodedPayload, Stage2Counters, Stage2Pool};
#[cfg(feature = "__test-helpers")]
pub use status::StatusParseError;
