//! gRPC response-stream helpers on [`MarketDataClient`].
//!
//! MDDS RPCs are server-streaming: each call yields a
//! [`crate::grpc::ServerStreaming`] of `ResponseData` messages whose
//! payloads are zstd-compressed `DataTable` chunks. Two collection
//! strategies are provided:
//!
//! - [`collect_stream`](MarketDataClient::collect_stream) (crate-private) — drains
//!   the stream into a single merged `DataTable`. Used by the generated list
//!   and parsed endpoint macros where the caller expects a finite result.
//! - [`for_each_chunk`](MarketDataClient::for_each_chunk) (public) — streams each
//!   chunk into a caller-supplied closure without materializing every row.
//!   Used by the generated streaming builders and public enough for callers
//!   processing multi-million-row responses.

use std::future::Future;
use std::ops::ControlFlow;

use crate::util::stream_ext::StreamNextExt;

use crate::decode;
use crate::error::Error;
use crate::grpc::ServerStreaming;
use crate::proto;

use super::client::MarketDataClient;

pub(crate) fn chunk_columns<T: crate::columns::WireColumns>(
    table: &proto::DataTable,
) -> crate::columns::ColumnPresence {
    let header_refs: Vec<&str> = table.headers.iter().map(String::as_str).collect();
    let columns = T::present_columns(&header_refs);
    if columns.contains("symbol") {
        return columns;
    }
    match super::decode::extract::response_symbol(table) {
        super::decode::extract::ResponseSymbol::Constant(symbol) => columns.with_symbol(symbol),
        super::decode::extract::ResponseSymbol::PerRow(symbols) => columns.with_symbols(symbols),
        super::decode::extract::ResponseSymbol::Absent => columns,
    }
}

impl MarketDataClient {
    /// Collect all streamed `ResponseData` chunks into a single `DataTable`.
    ///
    /// MDDS returns server-streaming responses where each chunk is a zstd-
    /// compressed `DataTable`. This helper decompresses, decodes, and merges
    /// all chunks into one contiguous table.
    ///
    /// Pre-allocates the row buffer based on the `original_size` hint from the
    /// first response, reducing reallocations for large responses.
    ///
    /// For truly large responses (millions of rows), prefer [`for_each_chunk`]
    /// which processes each chunk without materializing all rows in memory.
    ///
    /// [`for_each_chunk`]: Self::for_each_chunk
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when a chunk fails to decompress or decode, when a
    /// chunk exceeds the channel's `max_message_size` ceiling, or when
    /// mid-stream headers drift from the first chunk's schema
    /// (`decode::DecodeError::ChunkHeaderDrift`).
    pub(crate) async fn collect_stream(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
    ) -> Result<proto::DataTable, Error> {
        collect_stream_table(stream).await
    }

    /// Process streamed responses chunk-by-chunk without materializing all rows.
    ///
    /// Each gRPC `ResponseData` message is decoded independently and passed to
    /// the callback as `(headers, rows)`. This keeps peak memory proportional to
    /// a single chunk rather than the entire result set — critical for endpoints
    /// that return millions of rows (e.g. full-day trade history).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // `ignore` here because the example needs a live authenticated
    /// // `MarketDataClient` to open a server-streaming gRPC channel — no
    /// // in-process fixture can stand in.
    /// let request = /* build your gRPC request */;
    /// // Bind the channel lease so its pre-dispatch reservation
    /// // stays committed across the `.await`. Deref coercion from
    /// // `&ChannelLease` to `&Channel` satisfies the stub signature.
    /// let lease = client.channel();
    /// let stream = crate::proto::beta_theta_terminal::get_stock_history_trade(
    ///     &lease,
    ///     request,
    /// )
    /// .await?;
    ///
    /// let mut count = 0usize;
    /// client.for_each_chunk(stream, |_headers, rows| {
    ///     count += rows.len();
    /// }).await?;
    /// println!("processed {count} rows without buffering them all");
    /// ```
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub async fn for_each_chunk<F>(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(&[String], &[proto::DataValueList]),
    {
        self.for_each_chunk_control(stream, |headers, rows| {
            f(headers, rows);
            ControlFlow::Continue(())
        })
        .await
    }

    pub(crate) async fn for_each_chunk_control<F>(
        &self,
        mut stream: ServerStreaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(&[String], &[proto::DataValueList]) -> ControlFlow<()>,
    {
        // Preserve first-chunk headers across all chunks, matching
        // collect_stream behavior. Reject any mid-stream chunk whose
        // non-empty headers disagree with the first-chunk schema:
        // accepting them silently would let the callback read columns
        // under the wrong names.
        let mut saved_headers: Option<Vec<String>> = None;
        let mut chunk_index: usize = 0;
        let max_message_size = stream.max_message_size();
        while let Some(response) = stream.next().await {
            let response = response?;
            let table =
                decode_chunk_checked(response, max_message_size, &mut saved_headers, chunk_index)?;
            let headers = if table.headers.is_empty() {
                saved_headers.as_deref().unwrap_or(&[])
            } else {
                &table.headers
            };
            if f(headers, &table.data_table).is_break() {
                break;
            }
            chunk_index += 1;
        }
        Ok(())
    }

    /// Async twin of [`for_each_chunk`](Self::for_each_chunk): identical
    /// decode, header-drift guard, and first-chunk-header preservation,
    /// but the per-chunk callback returns a future that is awaited
    /// before the next chunk is fetched.
    ///
    /// Unlike the sync path, the callback receives the chunk by value
    /// (`Vec<String>` headers, `Vec<proto::DataValueList>` rows). The
    /// buffer is freed after each callback either way, so moving it in
    /// keeps the caller's future free of any borrow held across its
    /// `.await` — the reason the sync path can lend a slice is that its
    /// callback returns before the next chunk is fetched, which an async
    /// callback cannot promise.
    ///
    /// Awaiting the callback in-line keeps delivery once-per-chunk and
    /// in order, and preserves the same backpressure as the sync path:
    /// the current chunk is dropped and no further chunk is pulled off
    /// the wire until the callback future resolves. Used by the Python
    /// SDK's `*_stream_async` terminals, where the callback offloads the
    /// GIL-bound user handler onto tokio's blocking pool so the async
    /// workers stay free.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure
    /// (including `decode::DecodeError::ChunkHeaderDrift`).
    pub async fn for_each_chunk_async<F, Fut>(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(Vec<String>, Vec<proto::DataValueList>) -> Fut,
        Fut: Future<Output = ()>,
    {
        self.for_each_chunk_async_control(stream, |headers, rows| {
            let fut = f(headers, rows);
            async move {
                fut.await;
                ControlFlow::Continue(())
            }
        })
        .await
    }

    pub(crate) async fn for_each_chunk_async_control<F, Fut>(
        &self,
        mut stream: ServerStreaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(Vec<String>, Vec<proto::DataValueList>) -> Fut,
        Fut: Future<Output = ControlFlow<()>>,
    {
        let mut saved_headers: Option<Vec<String>> = None;
        let mut chunk_index: usize = 0;
        let max_message_size = stream.max_message_size();
        while let Some(response) = stream.next().await {
            let response = response?;
            let proto::DataTable {
                headers,
                data_table,
            } = decode_chunk_checked(response, max_message_size, &mut saved_headers, chunk_index)?;
            // Backfill the preserved first-chunk schema onto a
            // headers-only chunk so the callback always sees the schema
            // its rows belong to, matching the sync path.
            let headers = if headers.is_empty() {
                saved_headers.clone().unwrap_or_default()
            } else {
                headers
            };
            if f(headers, data_table).await.is_break() {
                break;
            }
            chunk_index += 1;
        }
        Ok(())
    }

    /// Drain one server stream for the generated `.stream(handler)` path:
    /// parse each chunk with `parser`, hand the tick slice to the shared
    /// `handler` under its lock, and mark `delivered` once a non-empty
    /// chunk has reached it (the no-resume replay guard read by
    /// [`super::macros::run_streaming_retry_loop`]).
    ///
    /// Shared verbatim between the single-stream arm and each shard of a
    /// bulk-fetch fan-out — the handler `Mutex` is what lets concurrent
    /// shards forward into one user callback. The handler takes `&[T]`
    /// (raw rows) and never reads column presence, so no `ColumnPresence`
    /// is computed here — that per-chunk work exists only on the
    /// [`Self::deliver_chunk_ticks`] path, whose handler reads it. The
    /// first parse failure is terminal: it breaks the drain and surfaces
    /// after the stream is released.
    ///
    /// # Errors
    ///
    /// Propagates stream / decode errors from the chunk drain, or the
    /// first `parser` failure.
    pub(crate) async fn deliver_chunk_slices<T, E, P, F>(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
        parser: P,
        handler: &std::sync::Mutex<F>,
        delivered: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Error>
    where
        P: Fn(&proto::DataTable) -> Result<Vec<T>, E>,
        E: Into<Error>,
        F: FnMut(&[T]) + Send,
    {
        let mut decode_error: Option<Error> = None;
        let drain_result = self
            .for_each_chunk_control(stream, |headers, rows| {
                if decode_error.is_some() {
                    return ControlFlow::Break(());
                }
                let chunk_table = proto::DataTable {
                    headers: headers.to_vec(),
                    data_table: rows.to_vec(),
                };
                match parser(&chunk_table) {
                    Ok(ticks) => {
                        // The mutex is uncontended in steady state (chunks
                        // of one call chain arrive one at a time; shards
                        // of a fan-out serialize on it per chunk); a
                        // poisoned mutex only surfaces if a handler
                        // panicked mid-callback, which is already a hard
                        // error path.
                        if let Ok(mut h) = handler.lock() {
                            (*h)(&ticks);
                        }
                        // Only mark the stream as delivered once a chunk
                        // carried rows: an empty chunk (headers-only
                        // keepalive / terminator) hands the downstream no
                        // rows, so a refresh replay from chunk zero would
                        // duplicate nothing. Gating here keeps a
                        // recoverable `Unauthenticated` after only empty
                        // chunks from being forced terminal.
                        if !ticks.is_empty() {
                            delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        ControlFlow::Continue(())
                    }
                    Err(e) => {
                        decode_error = Some(e.into());
                        ControlFlow::Break(())
                    }
                }
            })
            .await;
        drain_result.and_then(|()| match decode_error {
            Some(e) => Err(e),
            None => Ok(()),
        })
    }

    /// [`Self::deliver_chunk_slices`] with the presence-carrying
    /// [`crate::columns::Ticks`] wrap the `stream_ticks` terminal hands
    /// its handler (the SDK bindings read the per-chunk column set).
    ///
    /// # Errors
    ///
    /// Same as [`Self::deliver_chunk_slices`].
    pub(crate) async fn deliver_chunk_ticks<T, E, P, F>(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
        parser: P,
        handler: &std::sync::Mutex<F>,
        delivered: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Error>
    where
        T: crate::columns::WireColumns,
        P: Fn(&proto::DataTable) -> Result<Vec<T>, E>,
        E: Into<Error>,
        F: FnMut(crate::columns::Ticks<T>) + Send,
    {
        let mut decode_error: Option<Error> = None;
        let drain_result = self
            .for_each_chunk_control(stream, |headers, rows| {
                if decode_error.is_some() {
                    return ControlFlow::Break(());
                }
                let chunk_table = proto::DataTable {
                    headers: headers.to_vec(),
                    data_table: rows.to_vec(),
                };
                match parser(&chunk_table) {
                    Ok(rows) => {
                        let columns = chunk_columns::<T>(&chunk_table);
                        let ticks = crate::columns::Ticks::new(rows, columns);
                        let delivered_nonempty = !ticks.is_empty();
                        if let Ok(mut h) = handler.lock() {
                            (*h)(ticks);
                        }
                        if delivered_nonempty {
                            delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        ControlFlow::Continue(())
                    }
                    Err(e) => {
                        decode_error = Some(e.into());
                        ControlFlow::Break(())
                    }
                }
            })
            .await;
        drain_result.and_then(|()| match decode_error {
            Some(e) => Err(e),
            None => Ok(()),
        })
    }

    /// Async twin of [`Self::deliver_chunk_slices`] for the
    /// `stream_async` terminal.
    ///
    /// The handler lock is a `tokio::sync::Mutex` held across the
    /// handler future's await: under a bulk-fetch fan-out, concurrent
    /// shard bands deliver through one shared handler, and only a guard
    /// that survives the await keeps handler executions from
    /// overlapping — the documented one-call-at-a-time contract (a
    /// `std::sync` guard cannot be held across an await). The chunk is
    /// parsed on the sync side and dropped before the handler future is
    /// awaited, preserving the chunk-freed-before-handler-runs bound; on
    /// a single stream the lock is uncontended and the behaviour is
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Same as [`Self::deliver_chunk_slices`].
    pub(crate) async fn deliver_chunk_slices_async<T, E, P, F, HFut>(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
        parser: P,
        handler: &tokio::sync::Mutex<F>,
        delivered: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Error>
    where
        P: Fn(&proto::DataTable) -> Result<Vec<T>, E>,
        E: Into<Error>,
        F: FnMut(&[T]) -> HFut + Send,
        HFut: Future<Output = ()> + Send,
    {
        let mut decode_error: Option<Error> = None;
        let drain_result = self
            .for_each_chunk_async_control(stream, |headers, rows| {
                // Synchronous section: parse the chunk. The handler runs
                // in the returned future, where the async lock can be
                // held across its await.
                let mut stop_stream = decode_error.is_some();
                let ticks = if stop_stream {
                    None
                } else {
                    let chunk_table = proto::DataTable {
                        headers,
                        data_table: rows,
                    };
                    match parser(&chunk_table) {
                        Ok(ticks) => Some(ticks),
                        Err(e) => {
                            decode_error = Some(e.into());
                            stop_stream = true;
                            None
                        }
                    }
                };
                async move {
                    if let Some(ticks) = ticks {
                        let mut h = handler.lock().await;
                        let user_fut = (*h)(&ticks);
                        // Mark delivered once the handler has taken a
                        // non-empty chunk, so a later transient cannot
                        // replay an already-delivered prefix. Empty
                        // chunks (headers-only keepalive) hand the
                        // handler no rows, so a replay would duplicate
                        // nothing.
                        if !ticks.is_empty() {
                            delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        // `HFut` is an independent type, so the handler
                        // future cannot borrow the slice: free the chunk
                        // before running the handler, keeping peak memory
                        // at one chunk per stream.
                        drop(ticks);
                        user_fut.await;
                    }
                    if stop_stream {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            })
            .await;
        drain_result.and_then(|()| match decode_error {
            Some(e) => Err(e),
            None => Ok(()),
        })
    }

    /// Async twin of [`Self::deliver_chunk_ticks`] for the
    /// `stream_ticks_async` terminal. Same across-the-await handler lock
    /// as [`Self::deliver_chunk_slices_async`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::deliver_chunk_slices`].
    pub(crate) async fn deliver_chunk_ticks_async<T, E, P, F, HFut>(
        &self,
        stream: ServerStreaming<proto::ResponseData>,
        parser: P,
        handler: &tokio::sync::Mutex<F>,
        delivered: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Error>
    where
        T: crate::columns::WireColumns,
        P: Fn(&proto::DataTable) -> Result<Vec<T>, E>,
        E: Into<Error>,
        F: FnMut(crate::columns::Ticks<T>) -> HFut + Send,
        HFut: Future<Output = ()> + Send,
    {
        let mut decode_error: Option<Error> = None;
        let drain_result = self
            .for_each_chunk_async_control(stream, |headers, rows| {
                let mut stop_stream = decode_error.is_some();
                let ticks = if stop_stream {
                    None
                } else {
                    let chunk_table = proto::DataTable {
                        headers,
                        data_table: rows,
                    };
                    match parser(&chunk_table) {
                        Ok(rows) => {
                            let columns = chunk_columns::<T>(&chunk_table);
                            Some(crate::columns::Ticks::new(rows, columns))
                        }
                        Err(e) => {
                            decode_error = Some(e.into());
                            stop_stream = true;
                            None
                        }
                    }
                };
                async move {
                    if let Some(ticks) = ticks {
                        let delivered_nonempty = !ticks.is_empty();
                        let mut h = handler.lock().await;
                        let user_fut = (*h)(ticks);
                        if delivered_nonempty {
                            delivered.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        user_fut.await;
                    }
                    if stop_stream {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                }
            })
            .await;
        drain_result.and_then(|()| match decode_error {
            Some(e) => Err(e),
            None => Ok(()),
        })
    }
}

/// Free-function body of [`MarketDataClient::collect_stream`]: drain all
/// streamed `ResponseData` chunks into one merged `DataTable`.
///
/// A free function because the receiver contributes nothing — every
/// input lives on the stream. Semantics are exactly the method's:
/// first-chunk header capture, mid-stream header-drift rejection,
/// `original_size`-capped pre-allocation, and "empty stream is a valid
/// empty table". The bulk-fetch shard driver collects its spawned
/// per-shard streams through [`collect_stream_typed`] instead, which
/// shares this chunk contract but parses each chunk as it lands.
///
/// # Errors
///
/// Same conditions as [`MarketDataClient::collect_stream`].
pub(crate) async fn collect_stream_table(
    mut stream: ServerStreaming<proto::ResponseData>,
) -> Result<proto::DataTable, Error> {
    let mut all_rows = Vec::new();
    let mut headers: Vec<String> = Vec::new();
    let mut chunk_index: usize = 0;

    let max_message_size = stream.max_message_size();

    while let Some(response) = stream.next().await {
        let response = response?;

        // Use original_size as a rough pre-allocation hint on the first chunk.
        // Each DataValueList row is ~64 bytes on average (header-dependent),
        // so original_size / 64 gives a reasonable row-count estimate.
        //
        // Cap the hint at `max_message_size` so a hostile
        // `original_size = i32::MAX` cannot inflate `all_rows`'
        // capacity past the channel's configured ceiling before the
        // decompression layer rejects the payload.
        if all_rows.is_empty() && response.original_size > 0 {
            let hint = usize::try_from(response.original_size).unwrap_or(0);
            all_rows.reserve(hint.min(max_message_size) / 64);
        }

        let table = decode_chunk(response, max_message_size)?;
        if headers.is_empty() {
            headers = table.headers;
        } else if !table.headers.is_empty() && table.headers != headers {
            // Mid-stream schema drift: surface the mismatch so
            // downstream decoders do not read columns under the wrong
            // names. Silently keeping the first chunk's headers and
            // piling later rows under them would mislabel the data.
            return Err(decode::DecodeError::ChunkHeaderDrift {
                chunk_index,
                first: headers.join(","),
                chunk: table.headers.join(","),
            }
            .into());
        }
        all_rows.extend(table.data_table);
        chunk_index += 1;
    }

    // An empty stream is valid (e.g. no trades on a holiday) — return an
    // empty DataTable instead of Error::NoData. Callers that need to
    // distinguish "no data" can check `table.data_table.is_empty()`.
    Ok(proto::DataTable {
        headers,
        data_table: all_rows,
    })
}

/// Typed twin of [`collect_stream_table`] for the buffered bulk-fetch
/// shard path: drain one band's response stream, running the endpoint's
/// `parser` on each chunk while the chunk is decode-hot, and accumulate
/// the typed rows — the band never materializes its proto table, so a
/// multi-million-row band costs one chunk of proto cells at a time
/// plus its typed rows.
///
/// Chunk semantics are exactly [`collect_stream_table`]'s: first-chunk
/// header capture, mid-stream header-drift rejection, first-chunk-header
/// backfill for the parser (the contract the streaming delivery paths
/// apply per chunk), `original_size`-capped pre-allocation, and "empty
/// stream is a valid empty band". On top of those it folds the band's
/// `root` (symbol) column chunk-by-chunk
/// ([`super::shard::RootColumn`]), so the merged frame's presence /
/// symbol come out identical to the single-stream path's whole-table
/// `response_symbol` pass without a second pass over the rows.
///
/// Runs inside a shard's `run_unary_retry_loop` attempt closure; all
/// accumulation state is local to one call, so a replayed attempt
/// starts from an empty band.
///
/// # Errors
///
/// Same conditions as [`collect_stream_table`], plus the first `parser`
/// failure (typed decode error), which is terminal for the attempt.
pub(crate) async fn collect_stream_typed<T, E, P>(
    mut stream: ServerStreaming<proto::ResponseData>,
    parser: P,
) -> Result<super::shard::TypedBand<T>, Error>
where
    P: Fn(&proto::DataTable) -> Result<Vec<T>, E>,
    E: Into<Error>,
{
    let mut collect = TypedCollect::default();
    let max_message_size = stream.max_message_size();
    while let Some(response) = stream.next().await {
        let response = response?;
        // Same first-chunk row-count hint as `collect_stream_table`
        // (~64 B per wire row), capped at the channel ceiling; the typed
        // vector holds one element per wire row.
        if collect.rows.is_empty() && response.original_size > 0 {
            let hint = usize::try_from(response.original_size).unwrap_or(0);
            collect.rows.reserve(hint.min(max_message_size) / 64);
        }
        let table = decode_chunk(response, max_message_size)?;
        collect.fold_chunk(table, &parser)?;
    }
    Ok(collect.into_band())
}

/// Accumulator behind [`collect_stream_typed`]: one shard band's typed
/// rows, response schema, and root-column fold.
///
/// The per-chunk step lives on this struct rather than inline in the
/// drain loop so the chunk contract — header capture / drift rejection,
/// parse-under-band-schema, root constancy fold, row append — is
/// unit-testable with synthetic chunks (`ServerStreaming` has no
/// in-memory constructor; see `streaming_decode_contract`).
struct TypedCollect<T> {
    headers: Vec<String>,
    /// Resolved once from the band schema (alias-aware, like
    /// `response_symbol`); `None` until headers arrive or when the
    /// schema has no root column.
    root_idx: Option<usize>,
    root: super::shard::RootColumn,
    rows: Vec<T>,
    chunk_index: usize,
}

impl<T> Default for TypedCollect<T> {
    fn default() -> Self {
        Self {
            headers: Vec::new(),
            root_idx: None,
            root: super::shard::RootColumn::default(),
            rows: Vec::new(),
            chunk_index: 0,
        }
    }
}

impl<T> TypedCollect<T> {
    /// Fold one decoded chunk: enforce the first-chunk header contract,
    /// parse the chunk under the band schema, fold its root cells while
    /// they are cache-hot, and append the typed rows.
    fn fold_chunk<P, E>(&mut self, mut table: proto::DataTable, parser: &P) -> Result<(), Error>
    where
        P: Fn(&proto::DataTable) -> Result<Vec<T>, E>,
        E: Into<Error>,
    {
        if self.headers.is_empty() {
            self.headers = std::mem::take(&mut table.headers);
            if !self.headers.is_empty() {
                let refs: Vec<&str> = self.headers.iter().map(String::as_str).collect();
                self.root_idx = super::decode::headers::find_header(&refs, "root");
            }
        } else if !table.headers.is_empty() && table.headers != self.headers {
            // Mid-stream schema drift: same rejection as
            // `collect_stream_table`, so downstream decoders never read
            // columns under the wrong names.
            return Err(decode::DecodeError::ChunkHeaderDrift {
                chunk_index: self.chunk_index,
                first: self.headers.join(","),
                chunk: table.headers.join(","),
            }
            .into());
        }
        self.chunk_index += 1;
        // Parse under the band schema (first-chunk headers backfilled
        // onto headers-only chunks). The headers move in and back out —
        // no per-chunk clone.
        let chunk = proto::DataTable {
            headers: std::mem::take(&mut self.headers),
            data_table: table.data_table,
        };
        let parsed = parser(&chunk).map_err(Into::into)?;
        if let Some(idx) = self.root_idx {
            if !chunk.data_table.is_empty() {
                self.root.observe_chunk(
                    self.rows.len(),
                    chunk
                        .data_table
                        .iter()
                        .map(|row| super::decode::extract::root_cell_text(row, idx)),
                );
            }
        }
        self.headers = chunk.headers;
        self.rows.extend(parsed);
        Ok(())
    }

    fn into_band(self) -> super::shard::TypedBand<T> {
        super::shard::TypedBand {
            headers: self.headers,
            rows: self.rows,
            root: self.root,
        }
    }
}

/// Decode one streamed `ResponseData` and apply the first-chunk header
/// contract shared by [`MarketDataClient::for_each_chunk`] and
/// [`MarketDataClient::for_each_chunk_async`]: record the first non-empty
/// header row into `saved_headers`, and reject a later chunk whose
/// non-empty headers drift from it
/// (`decode::DecodeError::ChunkHeaderDrift`). The caller resolves which
/// header slice to hand the callback (the chunk's own or the preserved
/// first-chunk schema) since that borrow cannot outlive this helper.
fn decode_chunk_checked(
    response: proto::ResponseData,
    max_message_size: usize,
    saved_headers: &mut Option<Vec<String>>,
    chunk_index: usize,
) -> Result<proto::DataTable, Error> {
    let table = decode_chunk(response, max_message_size)?;
    if saved_headers.is_none() && !table.headers.is_empty() {
        *saved_headers = Some(table.headers.clone());
    } else if let Some(first) = saved_headers.as_deref() {
        if !table.headers.is_empty() && table.headers.as_slice() != first {
            return Err(decode::DecodeError::ChunkHeaderDrift {
                chunk_index,
                first: first.join(","),
                chunk: table.headers.join(","),
            }
            .into());
        }
    }
    Ok(table)
}

/// Decode a single `ResponseData` chunk (zstd decompress + `DataTable`
/// decode) inline on the caller's task, keeping the chunk on its
/// producing connection and avoiding cross-thread hand-off at every
/// production-reachable concurrency, including multi-chunk streams.
fn decode_chunk(
    response: proto::ResponseData,
    max_message_size: usize,
) -> Result<proto::DataTable, Error> {
    let mut response = response;
    decode::decode_data_table_with_max(&mut response, max_message_size)
}

#[cfg(test)]
mod streaming_decode_contract {
    //! Contract pin: the chunk-by-chunk decode primitive that the
    //! generated `.stream(handler)` method on every parsed builder
    //! routes through must surface chunks ONE AT A TIME and never carry
    //! a per-chunk `proto::DataTable` past its handler invocation.
    //!
    //! The contract these tests pin is the structural property that
    //! distinguishes the streaming variant from the buffered
    //! `IntoFuture` path: the buffered path's `collect_stream` accumulates
    //! `Vec<DataValueList>` across every chunk (~64 bytes × row-count
    //! resident peak); the streaming primitive `for_each_chunk` keeps
    //! exactly one chunk live at a time, then drops it before fetching
    //! the next. A regression that re-introduced the row accumulator
    //! inside `for_each_chunk` would be invisible to the existing tick
    //! parsers (which are per-table pure) but would re-open the 6×
    //! memory amplification reported on `option_history_quote(QQQ, 1DTE,
    //! interval=tick, strike_range=5)`.
    //!
    //! These tests construct synthetic `proto::ResponseData` chunks
    //! and route them through the same `decode_chunk` primitive both
    //! `collect_stream` and `for_each_chunk` use, asserting:
    //!
    //! 1. Each chunk decodes to its own row set in isolation.
    //! 2. The total ticks observed across N chunks equals the sum of
    //!    per-chunk row counts (no double-counting from the retry shell).
    //! 3. The `max_message_size` ceiling is enforced on every chunk —
    //!    a hostile `original_size` cannot bypass the bound by riding
    //!    inside a streaming response.

    use super::*;
    use crate::error::DecompressErrorKind;
    use crate::proto;

    /// Build a `proto::ResponseData` carrying a `DataTable` of two-column
    /// rows (`symbol`, `count`) — schema-agnostic shape that the
    /// `extract_text_column` decoder can read back without going through
    /// the per-tick parsers (which require the v3-canonical column
    /// names).
    fn make_chunk(rows: &[(&str, i64)]) -> proto::ResponseData {
        let table = proto::DataTable {
            headers: vec!["symbol".to_string(), "count".to_string()],
            data_table: rows
                .iter()
                .map(|(sym, count)| proto::DataValueList {
                    values: vec![
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Text(sym.to_string())),
                        },
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Number(*count)),
                        },
                    ],
                })
                .collect(),
        };
        let encoded = prost::Message::encode_to_vec(&table);
        proto::ResponseData {
            compression_description: Some(proto::CompressionDescription {
                algo: proto::CompressionAlgo::None as i32,
                level: 0,
            }),
            original_size: 0,
            compressed_data: encoded,
        }
    }

    #[test]
    fn decode_chunk_decodes_each_chunk_in_isolation() {
        // Each chunk's owned `ResponseData` is consumed by value, the
        // working buffer is freed before the next chunk is fetched,
        // and the returned `DataTable` carries exactly the rows the
        // chunk encoded. Pins the per-chunk decode primitive — the
        // higher-level `for_each_chunk` peak-memory contract is
        // exercised by the integration tests in `tests/`.
        let chunks = vec![
            make_chunk(&[("AAPL", 1), ("MSFT", 2)]),
            make_chunk(&[("GOOG", 3)]),
            make_chunk(&[("NVDA", 4), ("AMD", 5), ("INTC", 6)]),
        ];
        let mut per_chunk_row_counts = Vec::new();
        let mut total_rows = 0_usize;
        let max = 4 * 1024 * 1024;
        for chunk in chunks {
            let table = decode_chunk(chunk, max).expect("inline decode");
            per_chunk_row_counts.push(table.data_table.len());
            total_rows += table.data_table.len();
        }
        assert_eq!(per_chunk_row_counts, vec![2, 1, 3]);
        assert_eq!(total_rows, 6);
    }

    #[test]
    fn decode_chunk_checked_preserves_first_headers_and_guards_drift() {
        // The header contract shared by `for_each_chunk` and
        // `for_each_chunk_async`: the first non-empty header row is
        // preserved, a later chunk with matching headers passes, and a
        // later chunk whose non-empty headers disagree is rejected as
        // `ChunkHeaderDrift`.
        let max = 4 * 1024 * 1024;
        let mut saved: Option<Vec<String>> = None;

        let first = decode_chunk_checked(make_chunk(&[("AAPL", 1)]), max, &mut saved, 0)
            .expect("first chunk decodes");
        assert_eq!(first.headers, vec!["symbol", "count"]);
        assert_eq!(saved, Some(vec!["symbol".to_string(), "count".to_string()]));

        // A matching-header chunk is accepted and leaves the saved
        // schema untouched.
        decode_chunk_checked(make_chunk(&[("MSFT", 2)]), max, &mut saved, 1)
            .expect("matching headers accepted");

        // A drifting-header chunk is rejected before the callback runs.
        let drift = proto::DataTable {
            headers: vec!["ticker".to_string(), "n".to_string()],
            data_table: vec![],
        };
        let encoded = prost::Message::encode_to_vec(&drift);
        let drift_chunk = proto::ResponseData {
            compression_description: Some(proto::CompressionDescription {
                algo: proto::CompressionAlgo::None as i32,
                level: 0,
            }),
            original_size: 0,
            compressed_data: encoded,
        };
        let err = decode_chunk_checked(drift_chunk, max, &mut saved, 2)
            .expect_err("drifting headers must be rejected");
        let Error::Decode {
            source: Some(src), ..
        } = &err
        else {
            panic!("drift must surface as Error::Decode, got {err:?}");
        };
        assert!(
            matches!(
                src.downcast_ref::<decode::DecodeError>(),
                Some(decode::DecodeError::ChunkHeaderDrift { chunk_index: 2, .. })
            ),
            "drift source must be ChunkHeaderDrift at chunk_index 2, got {src:?}"
        );
    }

    #[test]
    fn chunk_columns_project_wire_columns_and_symbol() {
        let table = proto::DataTable {
            headers: vec![
                "symbol".to_string(),
                "ms_of_day".to_string(),
                "sequence".to_string(),
                "price".to_string(),
                "date".to_string(),
            ],
            data_table: vec![proto::DataValueList {
                values: vec![
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Text("SPY".into())),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(34_200_000)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(1)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(50_125)),
                    },
                    proto::DataValue {
                        data_type: Some(proto::data_value::DataType::Number(20_260_701)),
                    },
                ],
            }],
        };
        let columns = chunk_columns::<crate::TradeTick>(&table);
        assert!(columns.contains("ms_of_day"));
        assert!(columns.contains("sequence"));
        assert!(columns.contains("price"));
        assert!(columns.contains("date"));
        assert!(!columns.contains("expiration"));
        assert!(!columns.contains("strike"));
        assert!(!columns.contains("right"));
        assert_eq!(columns.symbol(), Some("SPY"));
    }

    #[tokio::test]
    async fn for_each_chunk_async_awaits_each_chunk_once_in_order() {
        // Mirrors `decode_chunk_decodes_each_chunk_in_isolation` for the
        // async driver: each chunk's future is awaited to completion
        // before the next chunk is decoded (in-order, once-per-chunk,
        // chunk-freed-before-next backpressure). `ServerStreaming` has no
        // in-memory constructor, so we drive the same `decode_chunk_checked`
        // + await-per-chunk loop `for_each_chunk_async` runs.
        let chunks = vec![
            make_chunk(&[("AAPL", 1), ("MSFT", 2)]),
            make_chunk(&[("GOOG", 3)]),
            make_chunk(&[("NVDA", 4), ("AMD", 5), ("INTC", 6)]),
        ];
        let max = 4 * 1024 * 1024;
        let mut saved: Option<Vec<String>> = None;
        let mut order = Vec::new();
        // A shared counter the callback future bumps on entry and clears
        // on exit: if two chunks were ever in flight at once it would read
        // 2 and trip the assert. Sequential await-before-next keeps it at 1.
        let active = std::rc::Rc::new(std::cell::Cell::new(0u32));

        let f = |_headers: Vec<String>, rows: Vec<proto::DataValueList>| {
            let active = active.clone();
            let count = rows.len();
            async move {
                active.set(active.get() + 1);
                assert_eq!(active.get(), 1, "at most one chunk future in flight");
                active.set(active.get() - 1);
                count
            }
        };

        for (chunk_index, chunk) in chunks.into_iter().enumerate() {
            let proto::DataTable {
                headers,
                data_table,
            } = decode_chunk_checked(chunk, max, &mut saved, chunk_index).expect("decode");
            order.push(f(headers, data_table).await);
        }
        assert_eq!(order, vec![2, 1, 3]);
    }

    /// Bare `DataTable` chunk for driving `TypedCollect::fold_chunk`
    /// directly (the buffered shard collector's per-chunk step; the
    /// stream driver above it only decodes and hands chunks over).
    fn typed_chunk(headers: &[&str], rows: &[(&str, i64)]) -> proto::DataTable {
        proto::DataTable {
            headers: headers.iter().map(|s| (*s).to_string()).collect(),
            data_table: rows
                .iter()
                .map(|(sym, count)| proto::DataValueList {
                    values: vec![
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Text(sym.to_string())),
                        },
                        proto::DataValue {
                            data_type: Some(proto::data_value::DataType::Number(*count)),
                        },
                    ],
                })
                .collect(),
        }
    }

    /// Test parser: one typed row per wire row, carrying the `count`
    /// cell. Length-preserving like every generated tick parser.
    fn parse_counts(table: &proto::DataTable) -> Result<Vec<i64>, decode::DecodeError> {
        Ok(table
            .data_table
            .iter()
            .map(
                |row| match row.values.get(1).and_then(|v| v.data_type.as_ref()) {
                    Some(proto::data_value::DataType::Number(n)) => *n,
                    _ => -1,
                },
            )
            .collect())
    }

    #[test]
    fn typed_collect_parses_per_chunk_and_folds_root_constancy() {
        // The buffered shard collector must decode chunk-at-a-time (rows
        // parsed and appended per chunk, never a whole-band proto table)
        // and fold the root column's constancy the way the streaming
        // path's `chunk_columns` reads it per chunk.
        let mut collect = TypedCollect::default();
        collect
            .fold_chunk(
                typed_chunk(&["symbol", "count"], &[("SPY", 1), ("SPY", 2)]),
                &parse_counts,
            )
            .expect("first chunk folds");
        // A chunk that carries no headers parses under the preserved
        // first-chunk schema — the same backfill the streaming delivery
        // paths apply per chunk.
        collect
            .fold_chunk(typed_chunk(&[], &[("SPY", 3)]), &parse_counts)
            .expect("headerless chunk folds under the saved schema");
        let band = collect.into_band();
        assert_eq!(band.rows, vec![1, 2, 3]);
        assert_eq!(band.headers, vec!["symbol", "count"]);
        // The wire root column ("symbol" resolves via the shared header
        // alias) was constant: one broadcast value, no per-row
        // materialization.
        assert_eq!(
            band.root,
            crate::mdds::shard::RootColumn::Uniform(Some("SPY".into()))
        );
    }

    #[test]
    fn typed_collect_materializes_per_row_root_on_divergence() {
        let mut collect = TypedCollect::default();
        collect
            .fold_chunk(
                typed_chunk(&["symbol", "count"], &[("SPY", 1), ("SPY", 2)]),
                &parse_counts,
            )
            .expect("first chunk folds");
        collect
            .fold_chunk(
                typed_chunk(&["symbol", "count"], &[("QQQ", 3)]),
                &parse_counts,
            )
            .expect("divergent chunk folds");
        let band = collect.into_band();
        // The uniform prefix expanded, so per-row values stay aligned
        // with the typed rows — exactly what the whole-table
        // `response_symbol` pass would have produced.
        assert_eq!(
            band.root,
            crate::mdds::shard::RootColumn::PerRow(vec![
                Some("SPY".into()),
                Some("SPY".into()),
                Some("QQQ".into()),
            ])
        );
    }

    #[test]
    fn typed_collect_without_root_header_stays_unobserved() {
        let mut collect = TypedCollect::default();
        collect
            .fold_chunk(typed_chunk(&["ms", "count"], &[("x", 1)]), &parse_counts)
            .expect("chunk folds");
        assert_eq!(
            collect.into_band().root,
            crate::mdds::shard::RootColumn::Unobserved
        );
    }

    #[test]
    fn typed_collect_rejects_mid_stream_header_drift() {
        // Same rejection as `collect_stream_table` / the streaming
        // paths: a later chunk whose non-empty headers disagree with the
        // band schema fails before its rows are parsed.
        let mut collect = TypedCollect::default();
        collect
            .fold_chunk(
                typed_chunk(&["symbol", "count"], &[("SPY", 1)]),
                &parse_counts,
            )
            .expect("first chunk folds");
        let err = collect
            .fold_chunk(typed_chunk(&["ticker", "n"], &[("SPY", 2)]), &parse_counts)
            .expect_err("drifting headers must be rejected");
        let Error::Decode {
            source: Some(src), ..
        } = &err
        else {
            panic!("drift must surface as Error::Decode, got {err:?}");
        };
        assert!(
            matches!(
                src.downcast_ref::<decode::DecodeError>(),
                Some(decode::DecodeError::ChunkHeaderDrift { chunk_index: 1, .. })
            ),
            "drift source must be ChunkHeaderDrift at chunk_index 1, got {src:?}"
        );
    }

    #[test]
    fn typed_collect_surfaces_the_first_parser_failure() {
        let failing = |_: &proto::DataTable| -> Result<Vec<i64>, decode::DecodeError> {
            Err(decode::DecodeError::MissingRequiredHeader {
                header: "count",
                rows: 1,
                available: "symbol".to_string(),
            })
        };
        let mut collect = TypedCollect::default();
        let err = collect
            .fold_chunk(typed_chunk(&["symbol", "count"], &[("SPY", 1)]), &failing)
            .expect_err("parser failure is terminal for the chunk");
        assert!(
            matches!(err, Error::Decode { .. }),
            "expected a decode-class error, got {err:?}"
        );
    }

    #[test]
    fn max_message_size_ceiling_enforced_per_chunk() {
        // A hostile peer that sets `original_size = i32::MAX` on a
        // single chunk inside a streaming response cannot bypass the
        // ceiling — the per-chunk decode rejects it BEFORE allocation.
        // The `max_message_size` clamp applies on every chunk the
        // streaming primitive routes through, not just on the buffered
        // `collect_stream` path.
        let hostile = proto::ResponseData {
            compression_description: Some(proto::CompressionDescription {
                algo: proto::CompressionAlgo::Zstd as i32,
                level: 0,
            }),
            original_size: i32::MAX,
            compressed_data: vec![],
        };
        let err = decode_chunk(hostile, 4 * 1024 * 1024)
            .expect_err("hostile original_size must be rejected before alloc");
        assert!(matches!(
            err,
            Error::Decompress {
                kind: DecompressErrorKind::MessageTooLarge { .. },
                ..
            }
        ));
    }
}
