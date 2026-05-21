//! Async [`Stream`] adapter over an h2 server-streaming response body.
//!
//! [`ServerStreaming`] wraps an [`h2::RecvStream`] and the framing
//! [`Codec`] so callers see a typed `Stream<Item = Result<Resp,
//! ChannelError>>`. Inbound h2 DATA frames feed a [`BytesMut`]
//! accumulator; the codec pops one decoded `Resp` at a time. When the
//! body closes, trailers are awaited and parsed into a [`Status`];
//! any non-OK status surfaces as a [`ChannelError::Rpc`].
//!
//! The poll loop is written by hand on top of [`futures_core::Stream`]
//! rather than via `async_stream` to keep the dependency surface narrow
//! and the state machine inspectable.
//!
//! # Deadlines and GOAWAY
//!
//! A caller-supplied [`tokio::time::Instant`] cuts the entire RPC
//! (request, streaming, trailers); when the instant passes the next
//! poll surfaces [`ChannelError::DeadlineExceeded`] and drops the h2
//! stream (sending RST_STREAM to the server). h2 connection-level
//! `GOAWAY` frames surface as [`ChannelError::ConnectionClosed`]
//! distinct from a stream-level reset so connection-pool consumers
//! can recycle the channel rather than retry on the same one.
//!
//! # Cancellation
//!
//! Dropping the [`ServerStreaming`] drops the underlying
//! `h2::RecvStream`, which sends RST_STREAM cleanly. The caller does
//! not need to explicitly cancel.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Buf, BufMut, BytesMut};
use futures_core::Stream;
use h2::RecvStream;
use pin_project_lite::pin_project;
use tokio::time::{Instant, Sleep};

use super::channel::ChannelError;
use super::codec::Codec;
use super::decoder_pool::DecoderHandle;
use super::status::Status;

/// Boxed `Sleep` so [`ServerStreaming`] stays `Unpin`. The deadline
/// path takes a heap allocation per call; the non-deadline path is
/// allocation-free.
type BoxedSleep = Pin<Box<Sleep>>;

pin_project! {
    /// `Stream<Item = Result<Resp, ChannelError>>` over an h2 response body.
    ///
    /// Yields one decoded `Resp` per poll, then `Ok(Status)` translated
    /// to either stream-end (status OK) or [`ChannelError::Rpc`] (status
    /// non-OK). After the terminating poll, the stream returns `None`.
    pub struct ServerStreaming<Resp> {
        // `Some` for a normal response with an open body; `None` for a
        // trailers-only OK response (no DATA frames, no trailers HEADERS
        // frame) where the caller already classified the status from
        // the initial HEADERS and just needs a stream that yields
        // nothing. `RecvStream` is `Unpin` (its public `poll_data` /
        // `poll_trailers` take `&mut self`), so the field itself does
        // not need pin projection.
        body: Option<RecvStream>,
        codec: Codec<(), Resp>,
        // Accumulator for bytes that have arrived but not yet been
        // assembled into a full length-prefixed frame.
        buf: BytesMut,
        // Once the body has been observed to end, the next poll awaits
        // and parses the trailers exactly once. `state` keeps that
        // contract explicit instead of buried in a sentinel field.
        state: StreamState,
        // Optional per-call deadline. When `Some`, the boxed `Sleep`
        // is polled alongside the body each iteration; on elapse, the
        // stream surfaces `ChannelError::DeadlineExceeded` and closes.
        // Boxing keeps `ServerStreaming` `Unpin` so callers can drive
        // it via `StreamExt::next` without manual pinning.
        deadline: Option<BoxedSleep>,
        deadline_duration_ms: u64,
        // Drop guard for the channel's in-flight stream counter. Held
        // here so the counter decrements exactly when this stream
        // ends — whether by exhaustion, by an error, or by cancel
        // (drop). The pool reads the counter to skip saturated
        // channels.
        in_flight_token: Option<super::channel::InFlightToken>,
        // Decoder ring this stream's chunks route through for the
        // heavy zstd + protobuf decode. Set to the channel's
        // attached handle at request dispatch; remains `None` for
        // channels constructed without a decoder pool (unit-test
        // paths), in which case consumers fall back to inline
        // decode on the caller's tokio task.
        decoder: Option<DecoderHandle>,
    }
}

/// State machine for [`ServerStreaming::poll_next`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamState {
    /// DATA frames still arriving; codec consumes them as they land.
    Receiving,
    /// Body ended; trailers must be awaited before the stream can close.
    AwaitingTrailers,
    /// Trailers consumed (or an error already surfaced); stream is done.
    Closed,
}

impl<Resp> ServerStreaming<Resp>
where
    Resp: prost::Message + Default,
{
    /// Wrap an h2 `RecvStream` with an explicit [`Codec`]. The channel
    /// constructs the codec carrying its configured `max_message_size`
    /// so oversized response frames surface as `FrameTooLarge` rather
    /// than the codec module's hardcoded default.
    pub(crate) fn with_codec(body: RecvStream, codec: Codec<(), Resp>) -> Self {
        Self {
            body: Some(body),
            codec,
            buf: BytesMut::new(),
            state: StreamState::Receiving,
            deadline: None,
            deadline_duration_ms: 0,
            in_flight_token: None,
            decoder: None,
        }
    }

    /// Wrap an h2 `RecvStream` with a per-call deadline and an explicit
    /// [`Codec`]. The deadline covers the entire stream — incoming
    /// DATA frames, trailers, and all intermediate polls. When it
    /// elapses the stream yields [`ChannelError::DeadlineExceeded`]
    /// and drops the h2 stream. The codec carries the channel's
    /// configured per-frame ceiling so the response-side decode limit
    /// matches `DirectConfig::mdds.max_message_size`.
    pub(crate) fn with_deadline_and_codec(
        body: RecvStream,
        deadline: Duration,
        codec: Codec<(), Resp>,
    ) -> Self {
        let duration_ms = u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX);
        Self {
            body: Some(body),
            codec,
            buf: BytesMut::new(),
            state: StreamState::Receiving,
            deadline: Some(Box::pin(tokio::time::sleep_until(
                Instant::now() + deadline,
            ))),
            deadline_duration_ms: duration_ms,
            in_flight_token: None,
            decoder: None,
        }
    }

    /// Attach the in-flight stream drop guard captured at request
    /// dispatch. The token's [`Drop`] decrements the channel's
    /// in-flight stream counter when this stream ends, freeing the
    /// pool to route around the channel while the stream is open.
    #[must_use]
    pub(crate) fn with_in_flight_token(mut self, token: super::channel::InFlightToken) -> Self {
        self.in_flight_token = Some(token);
        self
    }

    /// Attach the dedicated decoder ring this stream's chunks should
    /// route through. The handle is exposed to consumers via
    /// [`Self::decoder`] so [`crate::mdds::MddsClient::collect_stream`]
    /// and [`crate::mdds::MddsClient::for_each_chunk`] can hand each
    /// `ResponseData` to a worker thread for zstd + protobuf decode
    /// instead of running it on the tokio reactor.
    #[must_use]
    pub(crate) fn with_decoder(mut self, decoder: DecoderHandle) -> Self {
        self.decoder = Some(decoder);
        self
    }

    /// Borrow the decoder handle attached at request dispatch, if
    /// any. Consumers route their per-chunk decode through this
    /// handle so the heavy zstd + `DataTable::decode` work runs on
    /// a dedicated thread; `None` means the stream was constructed
    /// without a pool wired up and the consumer must decode inline.
    #[must_use]
    pub fn decoder(&self) -> Option<&DecoderHandle> {
        self.decoder.as_ref()
    }

    /// Per-frame ceiling configured on this stream's codec. Mirrors
    /// `DirectConfig::mdds.max_message_size` and is propagated to the
    /// decompression layer so a hostile `ResponseData.original_size`
    /// cannot trigger a runaway allocation past this bound (see
    /// [`crate::mdds::decode::decompress_response`]).
    #[must_use]
    pub fn max_message_size(&self) -> usize {
        self.codec.max_message_size()
    }

    /// Build a stream that immediately yields `Ok(None)`. Used when the
    /// server emitted a trailers-only OK response (status=0 on the
    /// initial HEADERS frame, no DATA frames, no trailing HEADERS
    /// frame) — the caller already extracted the status and just
    /// needs a `Stream` that closes cleanly.
    pub(crate) fn already_closed() -> Self {
        Self {
            body: None,
            codec: Codec::new(),
            buf: BytesMut::new(),
            state: StreamState::Closed,
            deadline: None,
            deadline_duration_ms: 0,
            in_flight_token: None,
            decoder: None,
        }
    }
}

/// Classify an `h2::Error` into the matching [`ChannelError`] variant.
///
/// Connection-level failures (`GOAWAY` in either direction, IO errors
/// at the h2 layer) surface as [`ChannelError::ConnectionClosed`] so
/// pool consumers recycle the channel. Per-stream `RST_STREAM` (any
/// reason code) stays as [`ChannelError::H2Stream`] — the h2
/// connection itself is healthy and the next RPC can succeed on the
/// same channel; classifying it as connection-level would force the
/// pool to recycle a still-good channel and burn retry budgets.
///
/// HTTP/2 spec § 7 (Error Codes) is the canonical list of reason
/// codes; the per-stream / connection-level distinction here matches
/// the wire-level scope of each frame type.
fn classify_h2_error(e: &h2::Error) -> ChannelError {
    if e.is_go_away() || e.is_io() {
        ChannelError::ConnectionClosed(e.to_string())
    } else {
        // is_reset() (per-stream RST_STREAM) and everything else
        // (library protocol error, user error, bare Reason) — the
        // h2 connection itself survives.
        ChannelError::H2Stream(e.to_string())
    }
}

impl<Resp> Stream for ServerStreaming<Resp>
where
    Resp: prost::Message + Default,
{
    type Item = Result<Resp, ChannelError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        loop {
            // Drain any frames that already fit in the accumulator
            // before bothering the deadline / body.
            if *this.state != StreamState::Closed {
                let mut peek = this.buf.clone().freeze();
                match this.codec.decode(&mut peek) {
                    Ok(Some(msg)) => {
                        let consumed = this.buf.len() - peek.remaining();
                        this.buf.advance(consumed);
                        return Poll::Ready(Some(Ok(msg)));
                    }
                    Ok(None) => {
                        // Need more bytes — fall through to body poll.
                    }
                    Err(e) => {
                        *this.state = StreamState::Closed;
                        return Poll::Ready(Some(Err(e.into())));
                    }
                }
            }

            // Check the per-call deadline before either branch of the
            // body/trailers state machine. The deadline applies to the
            // entire RPC — request, body, trailers — so we test it
            // once per outer iteration.
            if *this.state != StreamState::Closed {
                if let Some(sleep) = this.deadline.as_mut() {
                    if sleep.as_mut().poll(cx).is_ready() {
                        *this.state = StreamState::Closed;
                        return Poll::Ready(Some(Err(ChannelError::DeadlineExceeded {
                            duration_ms: *this.deadline_duration_ms,
                        })));
                    }
                }
            }

            // Trailers-only OK paths construct via `already_closed()`
            // with `body = None`. They reach this branch on the first
            // poll and exit cleanly.
            let Some(body) = this.body.as_mut() else {
                return Poll::Ready(None);
            };

            match *this.state {
                StreamState::Receiving => match body.poll_data(cx) {
                    Poll::Ready(Some(Ok(data))) => {
                        // h2 flow-control accounting — release the
                        // capacity reserved for this DATA frame so
                        // the server can continue sending.
                        let len = data.len();
                        let _ = body.flow_control().release_capacity(len);
                        this.buf.put(data);
                        continue;
                    }
                    Poll::Ready(Some(Err(e))) => {
                        *this.state = StreamState::Closed;
                        return Poll::Ready(Some(Err(classify_h2_error(&e))));
                    }
                    Poll::Ready(None) => {
                        // Body closed. If the codec accumulator
                        // holds a partial frame, that is a wire
                        // violation — surface it after the trailer
                        // step so the caller sees the trailer state
                        // even on a short body.
                        *this.state = StreamState::AwaitingTrailers;
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
                StreamState::AwaitingTrailers => {
                    // h2's `poll_trailers` is documented hidden but
                    // public — it lets the adapter poll the trailers
                    // future in-place without allocating a Box per
                    // iteration. Reaching this branch consumes the
                    // trailers exactly once; the next state advance is
                    // always `Closed`.
                    match body.poll_trailers(cx) {
                        Poll::Ready(Ok(Some(trailers))) => {
                            *this.state = StreamState::Closed;
                            // If the accumulator still has bytes, the
                            // server closed mid-frame.
                            if !this.buf.is_empty() {
                                return Poll::Ready(Some(Err(ChannelError::Codec(
                                    super::codec::CodecError::Decode(
                                        "body closed mid-frame; accumulator non-empty".into(),
                                    ),
                                ))));
                            }
                            match Status::from_trailers(&trailers) {
                                Ok(status) if status.is_ok() => return Poll::Ready(None),
                                Ok(status) => {
                                    return Poll::Ready(Some(Err(ChannelError::Rpc { status })));
                                }
                                Err(e) => return Poll::Ready(Some(Err(e.into()))),
                            }
                        }
                        Poll::Ready(Ok(None)) => {
                            // Defense-in-depth: a trailers-only response
                            // is normally caught at the channel layer
                            // (where we still have the response head).
                            // If we reach this branch, the body closed
                            // without trailers AND the channel layer
                            // didn't classify the head — the only
                            // truthful classification we can offer
                            // here is `StatusParse::Missing`.
                            *this.state = StreamState::Closed;
                            return Poll::Ready(Some(Err(ChannelError::StatusParse(
                                super::status::StatusParseError::Missing,
                            ))));
                        }
                        Poll::Ready(Err(e)) => {
                            *this.state = StreamState::Closed;
                            return Poll::Ready(Some(Err(classify_h2_error(&e))));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                StreamState::Closed => return Poll::Ready(None),
            }
        }
    }
}
