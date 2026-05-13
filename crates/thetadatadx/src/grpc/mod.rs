//! In-house gRPC client built directly on the [`h2`] crate, gated
//! behind the `inhouse-grpc` Cargo feature.
//!
//! # Why this exists
//!
//! The MDDS code path is server-streaming gRPC over HTTP/2 + TLS with
//! prost-encoded protobuf payloads. The `tonic`-backed implementation
//! pays per-call cost for `tonic::Request`, `tower::Service`, `BoxBody`,
//! and `async-trait` dynamic dispatch. This module runs directly on
//! [`h2`] so the hot path is: encode prost → frame → send DATA → poll
//! response stream → decode frames → parse trailers. No tower stack,
//! no boxed bodies, no dyn dispatch.
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
//! # Module layout
//!
//! - [`Codec`] — prost encode/decode + 5-byte length-prefix framing.
//! - [`Status`] — HTTP/2 trailers parser (`grpc-status` + `grpc-message`).
//! - [`Channel`] — one HTTP/2 connection, driven by a background tokio
//!   task that lives as long as the channel.
//! - [`ServerStreaming`] — async [`futures_core::Stream`] adapter over
//!   the h2 response body that yields decoded `Resp` values frame by
//!   frame and surfaces the trailing [`Status`] at end of stream.
//! - [`endpoints`] — one method per server-streaming RPC. Currently
//!   carries [`stock_list_symbols`].
//!
//! The default code path remains the `tonic`-backed [`MddsClient`] in
//! [`crate::mdds`]. Enabling the `inhouse-grpc` feature compiles this
//! module and exposes [`stock_list_symbols`] / [`stock_list_symbols_via_tonic`]
//! for A/B comparison.
//!
//! [`MddsClient`]: crate::mdds::MddsClient

pub mod channel;
pub mod codec;
pub mod endpoints;
pub mod status;
pub mod stream;

pub use channel::{Channel, ChannelError};
pub use codec::{Codec, CodecError};
pub use endpoints::{stock_list_symbols, stock_list_symbols_via_tonic};
pub use status::{Status, StatusParseError};
pub use stream::ServerStreaming;
