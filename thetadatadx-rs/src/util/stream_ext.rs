//! Minimal `Stream::next` adapter, analogous to `futures::StreamExt::next`.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

/// Extension trait that adds `.next()` to any [`Stream`].
///
/// Mirrors the `next` method from `futures::StreamExt`. Provided in-crate
/// so the transport layer does not require a separate `StreamExt` dep for a
/// single method.
pub(crate) trait StreamNextExt: Stream {
    /// Returns a future that resolves to the next item produced by this stream,
    /// or `None` when the stream is exhausted.
    fn next(&mut self) -> NextFuture<'_, Self>
    where
        Self: Unpin,
    {
        NextFuture { stream: self }
    }
}

impl<T: Stream> StreamNextExt for T {}

/// Future returned by [`StreamNextExt::next`].
pub(crate) struct NextFuture<'a, S: ?Sized> {
    stream: &'a mut S,
}

impl<S: Stream + Unpin + ?Sized> Future for NextFuture<'_, S> {
    type Output = Option<S::Item>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.stream).poll_next(cx)
    }
}
