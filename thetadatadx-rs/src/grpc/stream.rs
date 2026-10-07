//! Async [`Stream`] adapter over a server-streaming gRPC response.
//!
//! [`ServerStreaming`] wraps the underlying stack's streaming response
//! so callers see a typed `Stream<Item = Result<Resp, ChannelError>>`:
//! every error is mapped through the module's classifier at the poll
//! boundary, so no third-party error type crosses out of `crate::grpc`.
//!
//! # Cancellation
//!
//! Dropping the [`ServerStreaming`] drops the underlying response
//! stream, which sends RST_STREAM cleanly. The caller does not need to
//! explicitly cancel.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

use super::channel::{classify_poll_panic, classify_status, ChannelError, InFlightToken};

/// `Stream<Item = Result<Resp, ChannelError>>` over a server-streaming
/// gRPC response.
///
/// Yields one decoded `Resp` per message frame. A non-OK terminal
/// status surfaces as [`ChannelError::Rpc`]; transport faults surface
/// through the module's classifier ([`ChannelError::ConnectionClosed`]
/// / [`ChannelError::H2Stream`]). After a terminal item, the stream
/// returns `None`.
pub struct ServerStreaming<Resp> {
    /// Underlying response stream. `tonic::Streaming` is `Unpin`
    /// (its body and decoder are boxed), so the wrapper polls it via
    /// `Pin::new` without pin projection.
    inner: tonic::Streaming<Resp>,
    /// Per-frame ceiling configured on the source channel; surfaced
    /// to consumers via [`Self::max_message_size`] so the payload
    /// decompression layer enforces the same bound.
    max_message_size: usize,
    /// Drop guard for the channel's in-flight stream counter. Held
    /// here so the counter decrements exactly when this stream ends —
    /// whether by exhaustion, by an error, or by cancel (drop). The
    /// pool reads the counter to skip saturated channels.
    _in_flight_token: InFlightToken,
    /// Fuse: set after a terminal item (error or end-of-stream) so
    /// subsequent polls return `None` without touching the inner
    /// stream again.
    closed: bool,
}

impl<Resp> ServerStreaming<Resp>
where
    Resp: prost::Message + Default,
{
    /// Wrap a streaming response. `max_message_size` is the source
    /// channel's per-frame ceiling; `token` is the channel's in-flight
    /// drop guard captured at dispatch.
    pub(crate) fn new(
        inner: tonic::Streaming<Resp>,
        max_message_size: usize,
        token: InFlightToken,
    ) -> Self {
        Self {
            inner,
            max_message_size,
            _in_flight_token: token,
            closed: false,
        }
    }

    /// Per-frame ceiling configured on this stream's source channel.
    /// Mirrors `DirectConfig::mdds.max_message_size` and is propagated
    /// to the decompression layer so a hostile
    /// `ResponseData.original_size` cannot trigger a runaway
    /// allocation past this bound (see
    /// [`crate::mdds::decode::decompress_response_with_max`]).
    #[must_use]
    pub fn max_message_size(&self) -> usize {
        self.max_message_size
    }
}

impl<Resp> Stream for ServerStreaming<Resp>
where
    Resp: prost::Message + Default,
{
    type Item = Result<Resp, ChannelError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Every field is `Unpin` (the inner streaming type boxes its
        // body and decoder), so `get_mut` is sound without pin
        // projection.
        let this = self.get_mut();

        if this.closed {
            return Poll::Ready(None);
        }

        // The underlying gRPC implementation panics while parsing
        // end-of-stream trailers whose `grpc-status-details-bin` value
        // is not valid base64. A malformed trailer from the wire must
        // not unwind into the consumer's task, so the inner poll runs
        // inside `catch_unwind`; a caught panic fuses the stream and
        // surfaces as the terminal undecodable-trailer status (see
        // [`classify_poll_panic`]). `AssertUnwindSafe` is sound here:
        // the fuse guarantees the inner stream is never polled again
        // after a caught panic.
        let poll = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Pin::new(&mut this.inner).poll_next(cx)
        }));
        match poll {
            Err(payload) => {
                this.closed = true;
                Poll::Ready(Some(Err(classify_poll_panic(payload))))
            }
            Ok(Poll::Ready(Some(Ok(msg)))) => Poll::Ready(Some(Ok(msg))),
            Ok(Poll::Ready(Some(Err(status)))) => {
                this.closed = true;
                Poll::Ready(Some(Err(classify_status(status))))
            }
            Ok(Poll::Ready(None)) => {
                this.closed = true;
                Poll::Ready(None)
            }
            Ok(Poll::Pending) => Poll::Pending,
        }
    }
}
