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
//! distinct from a stream-level reset so the source [`super::Channel`]
//! can swap in a fresh h2 session in place.
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

use bytes::{BufMut, BytesMut};
use futures_core::Stream;
use h2::RecvStream;
use tokio::time::{Instant, Sleep};

use super::channel::{classify_h2_error_ref, ChannelError, ReconnectHandle};
use super::codec::Codec;
use super::decoder_pool::DecoderHandle;
use super::status::Status;

/// Boxed `Sleep` so [`ServerStreaming`] stays `Unpin`. The deadline
/// path takes a heap allocation per call; the non-deadline path is
/// allocation-free.
type BoxedSleep = Pin<Box<Sleep>>;

/// `Stream<Item = Result<Resp, ChannelError>>` over an h2 response body.
///
/// Yields one decoded `Resp` per poll, then `Ok(Status)` translated
/// to either stream-end (status OK) or [`ChannelError::Rpc`] (status
/// non-OK). After the terminating poll, the stream returns `None`.
///
/// Every field is `Unpin` (`RecvStream` exposes `&mut self` polls; the
/// deadline `Sleep` lives behind `Pin<Box<Sleep>>`, which is itself
/// `Unpin`). The struct is therefore auto-`Unpin` and `poll_next` takes
/// the inner reference via `Pin::get_mut` rather than a pin-projection
/// macro.
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
    // Reconnect handle on the source channel. When the streaming
    // poll observes `ChannelError::ConnectionClosed` the handle's
    // `.trigger()` kicks the channel into a fresh h2 session
    // (single-flight, bounded backoff). `None` for streams
    // constructed without a source channel (`already_closed`,
    // unit-test fixtures); production paths always wire one in.
    reconnect: Option<ReconnectHandle>,
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
            // Seed at 64 KiB so the typical DATA-frame chunk fits
            // without a per-frame realloc-and-copy on the hot decode
            // path. Empty streams pay one allocation up front; high-
            // throughput streams stop paying it per chunk.
            buf: BytesMut::with_capacity(64 * 1024),
            state: StreamState::Receiving,
            deadline: None,
            deadline_duration_ms: 0,
            in_flight_token: None,
            decoder: None,
            reconnect: None,
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
            // Seed at 64 KiB; see `with_codec` above for the rationale.
            buf: BytesMut::with_capacity(64 * 1024),
            state: StreamState::Receiving,
            deadline: Some(Box::pin(tokio::time::sleep_until(
                Instant::now() + deadline,
            ))),
            deadline_duration_ms: duration_ms,
            in_flight_token: None,
            decoder: None,
            reconnect: None,
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
            reconnect: None,
        }
    }

    /// Attach a reconnect-trigger handle on the source channel. When
    /// the streaming poll observes
    /// [`ChannelError::ConnectionClosed`], the handle's
    /// `.trigger()` kicks the channel into an in-place reconnect
    /// (single-flight, bounded backoff) so the next RPC dispatched
    /// through the channel observes the fresh h2 session.
    ///
    /// Builder-style; takes `self` to keep the existing chained
    /// construction shape at request dispatch in
    /// [`super::channel::Channel::server_streaming_frame`].
    #[must_use]
    pub(crate) fn with_reconnect_handle(mut self, handle: ReconnectHandle) -> Self {
        self.reconnect = Some(handle);
        self
    }
}

/// Peek the 5-byte gRPC frame prefix in-place and return the total
/// frame size (header + payload) when the accumulator already holds a
/// full frame, `None` when more wire bytes are needed, or an error on
/// a malformed header.
///
/// Reads the header bytes directly off the `BytesMut` slice (no
/// allocation, no clone) and lets the outer poll loop detach exactly
/// one frame's worth via
/// `BytesMut::split_to` on the success branch — refcount-only.
///
/// Header layout (gRPC spec § 7.1):
///   * byte 0      — compressed flag (0 = identity, 1 = compressed)
///   * bytes 1..=4 — big-endian u32 payload length
///
/// Header-level rejection (compressed flag != 0, payload length >
/// `max_message_size`) surfaces as a [`super::codec::CodecError`]
/// before any frame detach so the caller's "Err ⇒ buf not consumed"
/// invariant holds.
fn peek_frame_length(
    buf: &bytes::BytesMut,
    max_message_size: usize,
) -> Result<Option<usize>, super::codec::CodecError> {
    if buf.len() < super::codec::FRAME_HEADER_LEN {
        return Ok(None);
    }
    let header = &buf[..super::codec::FRAME_HEADER_LEN];
    let compressed_flag = header[0];
    match compressed_flag {
        0 => {}
        1 => return Err(super::codec::CodecError::CompressionUnsupported),
        other => return Err(super::codec::CodecError::InvalidCompressedFlag(other)),
    }
    let payload_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if payload_len > max_message_size {
        return Err(super::codec::CodecError::FrameTooLarge {
            length: payload_len,
            max: max_message_size,
        });
    }
    let total = super::codec::FRAME_HEADER_LEN + payload_len;
    if buf.len() < total {
        return Ok(None);
    }
    Ok(Some(total))
}

impl<Resp> Stream for ServerStreaming<Resp>
where
    Resp: prost::Message + Default,
{
    type Item = Result<Resp, ChannelError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: every field of `ServerStreaming` is `Unpin` (see the
        // struct docstring), so the type is auto-`Unpin` and
        // `Pin::get_mut` is sound — no pin-projection macro needed.
        let this = self.get_mut();

        loop {
            // Drain any frames that already fit in the accumulator
            // before bothering the deadline / body.
            //
            // Issue #565 Tier 4: peek the 5-byte length prefix WITHOUT
            // cloning the entire accumulator. The previous implementation
            // did `this.buf.clone().freeze()` per poll — `BytesMut::clone`
            // is a deep copy (verified empirically: a 10 MiB accumulator
            // duplicates to a fresh 10 MiB allocation), so a single
            // chunked response of N MiB paid an O(polls × N) memory tax
            // on the decode path. The optimised path below reads the
            // prefix in-place, splits off exactly one frame's worth on
            // the success branch (Bytes::split_to is refcounted, zero-
            // copy), and stays inside the accumulator on the
            // need-more-bytes branch.
            if this.state != StreamState::Closed {
                match peek_frame_length(&this.buf, this.codec.max_message_size()) {
                    Ok(Some(frame_len)) => {
                        // Full frame already buffered — detach exactly
                        // `frame_len` bytes (refcount-only, no copy),
                        // hand them to the codec, and yield the
                        // decoded message. The codec's internal
                        // `Bytes::split_to(payload_len)` then peels off
                        // the payload from the framed prefix.
                        let mut frame = this.buf.split_to(frame_len).freeze();
                        match this.codec.decode(&mut frame) {
                            Ok(Some(msg)) => {
                                return Poll::Ready(Some(Ok(msg)));
                            }
                            Ok(None) => {
                                // Cannot happen — `peek_frame_length`
                                // returned `Some` so the codec has the
                                // bytes it needs. Defensive: surface
                                // an internal error.
                                this.state = StreamState::Closed;
                                return Poll::Ready(Some(Err(ChannelError::Codec(
                                    super::codec::CodecError::Decode(
                                        "internal: codec returned None on a sized frame".into(),
                                    ),
                                ))));
                            }
                            Err(e) => {
                                this.state = StreamState::Closed;
                                return Poll::Ready(Some(Err(e.into())));
                            }
                        }
                    }
                    Ok(None) => {
                        // Need more bytes — fall through to body poll.
                    }
                    Err(e) => {
                        this.state = StreamState::Closed;
                        return Poll::Ready(Some(Err(e.into())));
                    }
                }
            }

            // Check the per-call deadline before either branch of the
            // body/trailers state machine. The deadline applies to the
            // entire RPC — request, body, trailers — so we test it
            // once per outer iteration.
            if this.state != StreamState::Closed {
                if let Some(sleep) = this.deadline.as_mut() {
                    if sleep.as_mut().poll(cx).is_ready() {
                        this.state = StreamState::Closed;
                        return Poll::Ready(Some(Err(ChannelError::DeadlineExceeded {
                            duration_ms: this.deadline_duration_ms,
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

            match this.state {
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
                        this.state = StreamState::Closed;
                        let classified = classify_h2_error_ref(&e);
                        if matches!(classified, ChannelError::ConnectionClosed(_)) {
                            // Connection-level death observed mid-stream.
                            // Kick the source channel into an in-place
                            // reconnect so the next RPC dispatched
                            // through the channel lands on a fresh h2
                            // session. Idempotent under the channel's
                            // single-flight CAS.
                            if let Some(handle) = this.reconnect.as_ref() {
                                handle.trigger();
                            }
                        }
                        return Poll::Ready(Some(Err(classified)));
                    }
                    Poll::Ready(None) => {
                        // Body closed. If the codec accumulator
                        // holds a partial frame, that is a wire
                        // violation — surface it after the trailer
                        // step so the caller sees the trailer state
                        // even on a short body.
                        this.state = StreamState::AwaitingTrailers;
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
                            this.state = StreamState::Closed;
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
                            // Redundant guard: a trailers-only response
                            // is normally caught at the channel layer
                            // (where we still have the response head).
                            // If we reach this branch, the body closed
                            // without trailers AND the channel layer
                            // didn't classify the head — the only
                            // truthful classification we can offer
                            // here is `StatusParse::Missing`.
                            this.state = StreamState::Closed;
                            return Poll::Ready(Some(Err(ChannelError::StatusParse(
                                super::status::StatusParseError::Missing,
                            ))));
                        }
                        Poll::Ready(Err(e)) => {
                            this.state = StreamState::Closed;
                            let classified = classify_h2_error_ref(&e);
                            if matches!(classified, ChannelError::ConnectionClosed(_)) {
                                // Trailers-phase ConnectionClosed gets
                                // the same treatment as body-phase:
                                // kick the source channel into a fresh
                                // h2 session.
                                if let Some(handle) = this.reconnect.as_ref() {
                                    handle.trigger();
                                }
                            }
                            return Poll::Ready(Some(Err(classified)));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                StreamState::Closed => return Poll::Ready(None),
            }
        }
    }
}

#[cfg(test)]
mod peek_frame_length_tests {
    //! Issue #565 Tier 4 pin: the accumulator-peek primitive that
    //! replaced the per-poll `BytesMut::clone()` must:
    //!
    //! 1. Return `Ok(None)` when fewer than 5 bytes are buffered (the
    //!    header isn't yet readable).
    //! 2. Return `Ok(None)` when the header is buffered but the payload
    //!    isn't yet complete.
    //! 3. Return `Ok(Some(5 + payload_len))` when a full frame is ready.
    //! 4. Reject hostile prefixes (oversized length, compressed flag !=
    //!    0) WITHOUT mutating the accumulator — the outer poll's
    //!    "Err ⇒ buf not consumed" invariant depends on this.
    //!
    //! These are the structural contracts that distinguish the Tier 4
    //! zero-copy path from the previous deep-clone path. A regression
    //! that re-introduced `BytesMut::clone()` here would pass every
    //! integration test (the semantics are identical) but would
    //! silently re-impose the per-poll O(buf.len()) memory tax.
    use super::*;
    use bytes::{BufMut, BytesMut};

    fn header(payload_len: u32) -> [u8; 5] {
        let mut hdr = [0u8; 5];
        hdr[0] = 0; // identity flag
        hdr[1..5].copy_from_slice(&payload_len.to_be_bytes());
        hdr
    }

    #[test]
    fn returns_none_when_header_incomplete() {
        let mut buf = BytesMut::new();
        buf.put_slice(&[0u8, 0u8, 0u8]); // 3 of 5 header bytes
        assert!(matches!(peek_frame_length(&buf, 4 * 1024 * 1024), Ok(None)));
        assert_eq!(buf.len(), 3, "accumulator must not be consumed on Ok(None)");
    }

    #[test]
    fn returns_none_when_payload_incomplete() {
        let mut buf = BytesMut::new();
        buf.put_slice(&header(100));
        buf.put_slice(&[0u8; 50]); // 50 of 100 payload bytes
        assert!(matches!(peek_frame_length(&buf, 4 * 1024 * 1024), Ok(None)));
        assert_eq!(
            buf.len(),
            55,
            "accumulator must not be consumed on Ok(None)"
        );
    }

    #[test]
    fn returns_total_frame_length_when_full() {
        let mut buf = BytesMut::new();
        buf.put_slice(&header(100));
        buf.put_slice(&[0u8; 100]);
        match peek_frame_length(&buf, 4 * 1024 * 1024) {
            Ok(Some(total)) => assert_eq!(total, 5 + 100),
            other => panic!("expected Ok(Some(105)), got {other:?}"),
        }
        assert_eq!(buf.len(), 105, "peek must not consume on success");
    }

    #[test]
    fn rejects_oversized_payload_without_mutating_buf() {
        let mut buf = BytesMut::new();
        buf.put_slice(&header(10 * 1024 * 1024)); // 10 MiB claimed
        let before = buf.len();
        match peek_frame_length(&buf, 4 * 1024 * 1024) {
            Err(super::super::codec::CodecError::FrameTooLarge { length, max }) => {
                assert_eq!(length, 10 * 1024 * 1024);
                assert_eq!(max, 4 * 1024 * 1024);
            }
            other => panic!("expected FrameTooLarge, got {other:?}"),
        }
        assert_eq!(buf.len(), before, "buf must not be consumed on error");
    }

    #[test]
    fn rejects_compressed_flag_without_mutating_buf() {
        let mut buf = BytesMut::new();
        let mut hdr = header(10);
        hdr[0] = 1; // compressed flag
        buf.put_slice(&hdr);
        buf.put_slice(&[0u8; 10]);
        let before = buf.len();
        match peek_frame_length(&buf, 4 * 1024 * 1024) {
            Err(super::super::codec::CodecError::CompressionUnsupported) => {}
            other => panic!("expected CompressionUnsupported, got {other:?}"),
        }
        assert_eq!(buf.len(), before, "buf must not be consumed on error");
    }
}
