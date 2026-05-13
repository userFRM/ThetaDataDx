//! In-house gRPC client built directly on the [`h2`] crate, gated
//! behind the `inhouse-grpc` Cargo feature.
//!
//! The module exposes the framing codec, the HTTP/2 trailers-based
//! [`Status`], the [`Channel`] transport, and a server-streaming
//! adapter. The `tonic`-backed code path remains the default; enabling
//! the `inhouse-grpc` feature swaps the MDDS path onto this stack.

pub mod channel;
pub mod codec;
pub mod status;
pub mod stream;

pub use channel::{Channel, ChannelError};
pub use codec::{Codec, CodecError};
pub use status::{Status, StatusParseError};
pub use stream::ServerStreaming;
