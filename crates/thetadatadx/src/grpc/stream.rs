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

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::{Buf, BufMut, BytesMut};
use futures_core::Stream;
use h2::RecvStream;
use pin_project_lite::pin_project;

use super::channel::ChannelError;
use super::codec::Codec;
use super::status::Status;

pin_project! {
    /// `Stream<Item = Result<Resp, ChannelError>>` over an h2 response body.
    ///
    /// Yields one decoded `Resp` per poll, then `Ok(Status)` translated
    /// to either stream-end (status OK) or [`ChannelError::Rpc`] (status
    /// non-OK). After the terminating poll, the stream returns `None`.
    pub struct ServerStreaming<Resp> {
        #[pin]
        body: RecvStream,
        codec: Codec<(), Resp>,
        // Accumulator for bytes that have arrived but not yet been
        // assembled into a full length-prefixed frame.
        buf: BytesMut,
        // Once the body has been observed to end, the next poll awaits
        // and parses the trailers exactly once. `state` keeps that
        // contract explicit instead of buried in a sentinel field.
        state: StreamState,
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
    /// Wrap an h2 `RecvStream` in the typed stream adapter.
    pub(crate) fn new(body: RecvStream) -> Self {
        Self {
            body,
            codec: Codec::new(),
            buf: BytesMut::new(),
            state: StreamState::Receiving,
        }
    }
}

impl<Resp> Stream for ServerStreaming<Resp>
where
    Resp: prost::Message + Default,
{
    type Item = Result<Resp, ChannelError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // Drain any frames that already fit in the accumulator.
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

            match *this.state {
                StreamState::Receiving => {
                    match this.body.as_mut().poll_data(cx) {
                        Poll::Ready(Some(Ok(data))) => {
                            // h2 flow-control accounting — release the
                            // capacity reserved for this DATA frame so
                            // the server can continue sending.
                            let len = data.len();
                            let _ = this.body.flow_control().release_capacity(len);
                            this.buf.put(data);
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => {
                            *this.state = StreamState::Closed;
                            return Poll::Ready(Some(Err(ChannelError::H2Stream(e.to_string()))));
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
                    }
                }
                StreamState::AwaitingTrailers => {
                    // h2's `poll_trailers` is documented hidden but
                    // public — it lets the adapter poll the trailers
                    // future in-place without allocating a Box per
                    // iteration. Reaching this branch consumes the
                    // trailers exactly once; the next state advance is
                    // always `Closed`.
                    match this.body.poll_trailers(cx) {
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
                            *this.state = StreamState::Closed;
                            return Poll::Ready(Some(Err(ChannelError::StatusParse(
                                super::status::StatusParseError::Missing,
                            ))));
                        }
                        Poll::Ready(Err(e)) => {
                            *this.state = StreamState::Closed;
                            return Poll::Ready(Some(Err(ChannelError::H2Stream(e.to_string()))));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
                StreamState::Closed => return Poll::Ready(None),
            }
        }
    }
}
