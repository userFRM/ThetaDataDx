//! Async [`Stream`] adapter over a server-streaming gRPC response.
//!
//! [`ServerStreaming`] wraps the underlying stack's streaming response
//! so callers see a typed `Stream<Item = Result<Resp, ChannelError>>`:
//! every error is mapped through the module's classifier at the poll
//! boundary, so no third-party error type crosses out of `crate::grpc`.
//!
//! # Deadlines
//!
//! A caller-supplied deadline (threaded from
//! [`super::Channel::server_streaming_with_deadline`]) cuts the
//! streaming phase: when it elapses, the next poll surfaces
//! [`ChannelError::DeadlineExceeded`] and the stream closes, dropping
//! the underlying h2 stream (which sends RST_STREAM to the server).
//!
//! # Cancellation
//!
//! Dropping the [`ServerStreaming`] drops the underlying response
//! stream, which sends RST_STREAM cleanly. The caller does not need to
//! explicitly cancel.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use tokio::time::{Instant, Sleep};

use super::channel::{classify_status, ChannelError, InFlightToken};

/// Boxed `Sleep` so [`ServerStreaming`] stays `Unpin`. The deadline
/// path takes a heap allocation per call; the non-deadline path is
/// allocation-free.
type BoxedSleep = Pin<Box<Sleep>>;

/// `Stream<Item = Result<Resp, ChannelError>>` over a server-streaming
/// gRPC response.
///
/// Yields one decoded `Resp` per message frame. A non-OK terminal
/// status surfaces as [`ChannelError::Rpc`]; transport faults surface
/// through the module's classifier ([`ChannelError::ConnectionClosed`]
/// / [`ChannelError::H2Stream`] / [`ChannelError::DeadlineExceeded`]).
/// After a terminal item, the stream returns `None`.
pub struct ServerStreaming<Resp> {
    /// Underlying response stream. `tonic::Streaming` is `Unpin`
    /// (its body and decoder are boxed), so the wrapper polls it via
    /// `Pin::new` without pin projection.
    inner: tonic::Streaming<Resp>,
    /// Per-frame ceiling configured on the source channel; surfaced
    /// to consumers via [`Self::max_message_size`] so the payload
    /// decompression layer enforces the same bound.
    max_message_size: usize,
    /// Optional per-call deadline. When `Some`, the boxed `Sleep` is
    /// polled alongside the body each iteration; on elapse, the
    /// stream surfaces [`ChannelError::DeadlineExceeded`] and closes.
    deadline: Option<BoxedSleep>,
    deadline_duration_ms: u64,
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
            deadline: None,
            deadline_duration_ms: 0,
            _in_flight_token: token,
            closed: false,
        }
    }

    /// Attach a per-call deadline covering the remaining streaming
    /// phase. Builder-style; called at dispatch with the open phase's
    /// elapsed time already subtracted. `duration_ms` is the caller's
    /// ORIGINAL deadline so the surfaced error reports the budget the
    /// caller supplied, not the residual.
    #[must_use]
    pub(crate) fn with_deadline(mut self, remaining: Duration, duration_ms: u64) -> Self {
        self.deadline = Some(Box::pin(tokio::time::sleep_until(
            Instant::now() + remaining,
        )));
        self.deadline_duration_ms = duration_ms;
        self
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
        // body and decoder; the deadline `Sleep` is behind `Pin<Box>`),
        // so `get_mut` is sound without pin projection.
        let this = self.get_mut();

        if this.closed {
            return Poll::Ready(None);
        }

        // Check the per-call deadline before polling the body — the
        // deadline applies to the entire streaming phase.
        if let Some(sleep) = this.deadline.as_mut() {
            if sleep.as_mut().poll(cx).is_ready() {
                this.closed = true;
                return Poll::Ready(Some(Err(ChannelError::DeadlineExceeded {
                    duration_ms: this.deadline_duration_ms,
                })));
            }
        }

        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(msg))) => Poll::Ready(Some(Ok(msg))),
            Poll::Ready(Some(Err(status))) => {
                this.closed = true;
                let deadline_ms = this.deadline.is_some().then_some(this.deadline_duration_ms);
                Poll::Ready(Some(Err(classify_status(status, deadline_ms))))
            }
            Poll::Ready(None) => {
                this.closed = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
