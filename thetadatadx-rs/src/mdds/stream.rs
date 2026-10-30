//! gRPC response-stream helpers on [`MarketDataClient`].
//!
//! MDDS RPCs are server-streaming: each call yields a
//! [`crate::grpc::ServerStreaming`] of `ResponseData` messages whose
//! payloads are zstd-compressed `DataTable` chunks. Two collection
//! strategies are provided:
//!
//! - [`collect_stream`](MarketDataClient::collect_stream) (crate-private) — drains
//!   the stream into a single merged `DataTable`. Used by the buffered list
//!   and tick requests, where the caller expects a finite result.
//! - [`for_each_chunk`](MarketDataClient::for_each_chunk) (public) — streams each
//!   chunk into a caller-supplied closure without materializing every row.
//!   Used by the generated streaming builders and public enough for callers
//!   processing multi-million-row responses.

use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::util::stream_ext::StreamNextExt;

use crate::columns::{Ticks, WireColumns};
use crate::decode;
use crate::error::{Error, GrpcStatusKind};
use crate::grpc::{Channel, ChannelError, ServerStreaming};
use crate::proto;

use super::client::MarketDataClient;
use super::macros::{
    effective_deadline, run_streaming_retry_loop, run_unary_retry_loop, run_with_optional_deadline,
    warn_buffered_response_size,
};
use super::shard::{ChainSortKey, ShardBand, ShardPlan, ShardStreamFuture};

/// The wire query of one generated endpoint, with what the endpoint runtime
/// needs to send it.
///
/// Every builder hands its proto query to the runtime as this trait object,
/// so the request, retry, fan-out and delivery code below is compiled once
/// per tick type rather than once per endpoint. The macros implement it for
/// each endpoint's query type.
pub(crate) trait EndpointQuery: Send + Sync {
    /// The bulk-fetch plan for this query, `None` to send it on one stream.
    fn plan(&self, client: &MarketDataClient) -> Option<ShardPlan>;

    /// This query narrowed to one band of its plan; every other parameter
    /// is forwarded as issued.
    fn band(&self, band: &ShardBand) -> Box<dyn EndpointQuery>;

    /// Open this query's server stream on `channel` for one attempt.
    fn open<'c>(&self, channel: &'c Channel, query_info: proto::QueryInfo) -> OpenStream<'c>;
}

/// The open phase of one attempt, as [`EndpointQuery::open`] returns it.
pub(crate) type OpenStream<'c> = Pin<
    Box<
        dyn Future<Output = Result<ServerStreaming<proto::ResponseData>, ChannelError>> + Send + 'c,
    >,
>;

/// A tick type's response parser, as the tick table names it.
pub(crate) type TickParser<T> = fn(&proto::DataTable) -> Result<Vec<T>, decode::DecodeError>;

/// How long a flat-file download waits for the service's next message. A
/// download that is making progress sends chunks far more often, so a longer
/// silence means the server has stopped sending on a live connection, which
/// the connection's keepalive never notices.
const FLAT_FILE_IDLE: Duration = Duration::from_secs(60);

/// Await `next`, failing once `idle` passes without it. The failure is a
/// `DeadlineExceeded` status, as a gRPC client reports its own deadline, so
/// the retry loop treats the stall as transient and reconnects.
async fn within_idle<T>(idle: Option<Duration>, next: impl Future<Output = T>) -> Result<T, Error> {
    let Some(idle) = idle else {
        return Ok(next.await);
    };
    tokio::time::timeout(idle, next)
        .await
        .map_err(|_| Error::Grpc {
            kind: GrpcStatusKind::DeadlineExceeded,
            message: format!("the stream sent nothing for {} s", idle.as_secs()),
            retry_after: None,
            http_status_code: None,
        })
}

/// The chunk callback of a streamed call: the builder's handler, taking each
/// chunk by value and returning whether to go on. A synchronous handler
/// returns a ready future.
pub(crate) type ChunkCallback<'h, T> =
    dyn FnMut(Ticks<T>) -> Pin<Box<dyn Future<Output = ControlFlow<()>> + Send + 'h>> + Send + 'h;

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
        mut stream: ServerStreaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(&[String], &[proto::DataValueList]),
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
            f(headers, &table.data_table);
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
        self.for_each_chunk_async_control(stream, None, |headers, rows| {
            let fut = f(headers, rows);
            async move {
                fut.await;
                ControlFlow::Continue(())
            }
        })
        .await
    }

    /// `idle`, when set, bounds the wait for each chunk; see [`within_idle`].
    pub(crate) async fn for_each_chunk_async_control<F, Fut>(
        &self,
        mut stream: ServerStreaming<proto::ResponseData>,
        idle: Option<Duration>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(Vec<String>, Vec<proto::DataValueList>) -> Fut,
        Fut: Future<Output = ControlFlow<()>>,
    {
        let mut saved_headers: Option<Vec<String>> = None;
        let mut chunk_index: usize = 0;
        let max_message_size = stream.max_message_size();
        while let Some(response) = within_idle(idle, stream.next()).await? {
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

    /// Stream one v3 flat file as the vendor sends it: each chunk's column
    /// headers and its cells, unconverted, so a caller can write the file
    /// exactly as the terminal does (the vendor sends flat-file prices as
    /// decimal text, and the text is the value).
    ///
    /// `endpoint` is the registry name, such as `stock_flat_file_quote`. A
    /// whole-universe file takes as long as it takes, so there is no
    /// deadline; a `Break` from `handler` ends the download. A service that
    /// sends nothing for 60 s fails the attempt as a stall. Transient
    /// failures, stalls included, are retried only until the first rows
    /// reach `handler`.
    ///
    /// # Errors
    ///
    /// Returns an error for an endpoint that is not a flat file, and on
    /// network, authentication or decode failure.
    #[doc(hidden)]
    pub async fn stream_flat_file<F, Fut>(
        &self,
        endpoint: &str,
        date: &str,
        handler: F,
    ) -> Result<(), Error>
    where
        F: FnMut(Vec<String>, Vec<proto::DataValueList>) -> Fut,
        Fut: Future<Output = ControlFlow<()>>,
    {
        macro_rules! request {
            ($snap:ident, $grpc:ident, $req:ident, $query:ident) => {{
                let request = proto::$req {
                    query_info: Some(self.build_query_info($snap.uuid.clone())),
                    params: Some(proto::$query {
                        date: super::wire_semantics::normalize_date(date),
                    }),
                };
                let lease = self.channel();
                within_idle(
                    Some(FLAT_FILE_IDLE),
                    crate::proto::beta_theta_terminal::$grpc(&lease, request),
                )
                .await?
            }};
        }
        let _permit = self.acquire_request_permit().await?;
        let handler = tokio::sync::Mutex::new(handler);
        let delivered = std::sync::atomic::AtomicBool::new(false);
        let (handler, delivered) = (&handler, &delivered);
        super::macros::run_streaming_retry_loop(
            self.session(),
            &self.config().retry,
            "stream_flat_file",
            delivered,
            |snap| async move {
                let stream = match endpoint {
                    "option_flat_file_eod" => request!(
                        snap,
                        get_option_flat_file_eod,
                        OptionFlatFileEodRequest,
                        OptionFlatFileEodRequestQuery
                    ),
                    "option_flat_file_trade_quote" => request!(
                        snap,
                        get_option_flat_file_trade_quote,
                        OptionFlatFileTradeQuoteRequest,
                        OptionFlatFileTradeQuoteRequestQuery
                    ),
                    "option_flat_file_open_interest" => request!(
                        snap,
                        get_option_flat_file_open_interest,
                        OptionFlatFileOpenInterestRequest,
                        OptionFlatFileOpenInterestRequestQuery
                    ),
                    "option_flat_file_quote" => request!(
                        snap,
                        get_option_flat_file_quote,
                        OptionFlatFileQuoteRequest,
                        OptionFlatFileQuoteRequestQuery
                    ),
                    "stock_flat_file_eod" => request!(
                        snap,
                        get_stock_flat_file_eod,
                        StockFlatFileEodRequest,
                        StockFlatFileEodRequestQuery
                    ),
                    "stock_flat_file_trade_quote" => request!(
                        snap,
                        get_stock_flat_file_trade_quote,
                        StockFlatFileTradeQuoteRequest,
                        StockFlatFileTradeQuoteRequestQuery
                    ),
                    "stock_flat_file_quote" => request!(
                        snap,
                        get_stock_flat_file_quote,
                        StockFlatFileQuoteRequest,
                        StockFlatFileQuoteRequestQuery
                    ),
                    "index_flat_file_eod" => request!(
                        snap,
                        get_index_flat_file_eod,
                        IndexFlatFileEodRequest,
                        IndexFlatFileEodRequestQuery
                    ),
                    other => {
                        return Err(Error::config_internal(format!(
                            "{other} is not a flat-file endpoint"
                        )));
                    }
                }
                .map_err(|e| -> Error { e.into() })?;
                self.for_each_chunk_async_control(
                    stream,
                    Some(FLAT_FILE_IDLE),
                    |headers, rows| async move {
                        if !rows.is_empty() {
                            delivered.store(true, std::sync::atomic::Ordering::Release);
                        }
                        (handler.lock().await)(headers, rows).await
                    },
                )
                .await
            },
        )
        .await
    }

    /// The body of every list endpoint: send `query` and return the text of
    /// its `column`, in the server's row order.
    ///
    /// `deadline` is the call's own, `None` for the configured default.
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication or parsing failure, and
    /// [`Error::Timeout`] when the deadline elapses first.
    pub(crate) async fn fetch_list(
        &self,
        endpoint: &'static str,
        deadline: Option<std::time::Duration>,
        query: &dyn EndpointQuery,
        column: &str,
    ) -> Result<Vec<String>, Error> {
        let deadline = effective_deadline(deadline, self.config().market_data.request_timeout_secs);
        run_with_optional_deadline(deadline, async move {
            tracing::debug!(endpoint, "gRPC request");
            metrics::counter!("thetadatadx.grpc.requests", "endpoint" => endpoint).increment(1);
            let started = std::time::Instant::now();
            let _permit = self.acquire_request_permit().await?;
            let table = run_unary_retry_loop(
                self.session(),
                &self.config().retry,
                endpoint,
                |snap| async move {
                    // The lease lives across the open, so the pool's picker
                    // counts the pending open on its channel.
                    let lease = self.channel();
                    let stream = query
                        .open(&lease, self.build_query_info(snap.uuid))
                        .await
                        .map_err(Error::from)?;
                    self.collect_stream(stream).await
                },
            )
            .await?;
            metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => endpoint)
                .record(started.elapsed().as_secs_f64() * 1_000.0);
            // The server's row order is the contract, as on the terminal.
            Ok(decode::extract_text_column(&table, column)
                .into_iter()
                .flatten()
                .collect())
        })
        .await
    }

    /// The buffered `.await` of every tick builder: send `query` and parse
    /// the whole response into one [`Ticks`].
    ///
    /// `deadline` is the builder's own, `None` for the configured default.
    /// A `query` the builder could not build is reported once the request
    /// has been counted. Under a bulk-fetch plan every band is its own
    /// top-level request, and the typed bands merge into the single
    /// stream's row order (see `mdds/shard.rs`).
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication or parsing failure, and
    /// [`Error::Timeout`] when the deadline elapses first.
    pub(crate) async fn fetch_ticks<T>(
        &self,
        endpoint: &'static str,
        deadline: Option<std::time::Duration>,
        query: Result<Box<dyn EndpointQuery>, Error>,
        parse: TickParser<T>,
    ) -> Result<Ticks<T>, Error>
    where
        T: WireColumns + ChainSortKey + Clone + Send + 'static,
    {
        let deadline = effective_deadline(deadline, self.config().market_data.request_timeout_secs);
        run_with_optional_deadline(deadline, async move {
            tracing::debug!(endpoint, "gRPC request");
            metrics::counter!("thetadatadx.grpc.requests", "endpoint" => endpoint).increment(1);
            let started = std::time::Instant::now();
            let query = query?;
            let ticks = match query.plan(self) {
                Some(plan) => {
                    // One independent top-level request per band. Each
                    // spawned task acquires its OWN request permit; this
                    // driver holds none while awaiting them, so the fan-out
                    // can never deadlock the pool-sized semaphore (worst case
                    // the bands serialize through it).
                    let mut tasks = Vec::with_capacity(plan.bands.len());
                    for band in &plan.bands {
                        let query = query.band(band);
                        let dispatch = self.shard_dispatch();
                        // The span tags every event of the band's task, the
                        // retry-sleep warnings included, with its window.
                        let band_span = super::shard::band_span(band);
                        tasks.push(super::shard::spawn_shard(tracing::Instrument::instrument(
                            async move {
                                let band_started = std::time::Instant::now();
                                // Set as soon as any chunk parses to rows and
                                // kept across attempts, so the join can refuse
                                // to fold a rows-then-NotFound band to empty.
                                let band_produced = AtomicBool::new(false);
                                let outcome = async {
                                    // Plain wait, not `acquire_request_permit`:
                                    // `auto_plan` declines any pull issued inside
                                    // a delivery handler, so a band only ever
                                    // waits on permits whose holders make
                                    // independent progress.
                                    let _permit =
                                        dispatch.semaphore.acquire().await.map_err(|_| {
                                            Error::config_internal("request semaphore closed")
                                        })?;
                                    let (dispatch, query, band_produced) =
                                        (&dispatch, &*query, &band_produced);
                                    run_unary_retry_loop(
                                        &dispatch.session,
                                        &dispatch.retry,
                                        endpoint,
                                        |snap| async move {
                                            let lease = dispatch.channel();
                                            let stream = query
                                                .open(&lease, dispatch.query_info(snap.uuid))
                                                .await
                                                .map_err(Error::from)?;
                                            // The rows accumulate inside the
                                            // attempt, so a band re-fetched after
                                            // a mid-collect transient starts
                                            // empty and the replay cannot
                                            // duplicate rows.
                                            collect_stream_typed(stream, parse, band_produced).await
                                        },
                                    )
                                    .await
                                }
                                .await;
                                if let Ok(typed_band) = &outcome {
                                    tracing::debug!(
                                        rows = typed_band.rows.len(),
                                        elapsed_ms = band_started.elapsed().as_millis() as u64,
                                        "shard band finished"
                                    );
                                }
                                (band_produced.load(Ordering::Relaxed), outcome)
                            },
                            band_span,
                        )));
                    }
                    // Join in band order (an empty band folds to zero rows),
                    // then merge the typed bands into the output row order.
                    // Any other band error fails the whole query, as a single
                    // stream's error would.
                    let ticks = super::shard::merge_typed_in_order(
                        super::shard::join_shards(tasks).await?,
                    )?;
                    metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => endpoint)
                        .record(started.elapsed().as_secs_f64() * 1_000.0);
                    ticks
                }
                None => {
                    // The permit releases as soon as the stream is drained,
                    // before the response is parsed.
                    let table = {
                        let _permit = self.acquire_request_permit().await?;
                        let query = &*query;
                        run_unary_retry_loop(
                            self.session(),
                            &self.config().retry,
                            endpoint,
                            |snap| async move {
                                let lease = self.channel();
                                let stream = query
                                    .open(&lease, self.build_query_info(snap.uuid))
                                    .await
                                    .map_err(Error::from)?;
                                self.collect_stream(stream).await
                            },
                        )
                        .await?
                    };
                    metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => endpoint)
                        .record(started.elapsed().as_secs_f64() * 1_000.0);
                    // Strict decode: a type mismatch in any cell fails the call.
                    let rows = parse(&table)?;
                    // The columns this response's wire carried and the root
                    // each row belongs to, so the frame projects to the
                    // terminal's exact column set.
                    Ticks::new(rows, chunk_columns::<T>(&table))
                }
            };
            // The wrong-API-for-this-workload signal, once per request, after
            // the rows materialized.
            warn_buffered_response_size(
                endpoint,
                ticks.len(),
                std::mem::size_of::<T>(),
                self.config().market_data.warn_on_buffered_threshold_bytes,
            );
            Ok(ticks)
        })
        .await
    }

    /// The streamed body of every tick builder (`stream`, `stream_async` and
    /// their binding terminals) and of every raw stream: send `query` and
    /// hand each chunk, parsed, to `callback`.
    ///
    /// `deadline` and `query` are as in [`Self::fetch_ticks`]. The callback
    /// runs one chunk at a time, and a `Break` from it ends the whole call,
    /// every band of a fan-out included. Under a bulk-fetch plan the bands
    /// stream concurrently, each its own top-level request, and their chunks
    /// reach the callback in arrival order (see `mdds/shard.rs`).
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication or parsing failure,
    /// [`Error::PartialShardFetch`] when some bands of a fan-out failed after
    /// others delivered, and [`Error::Timeout`] when the deadline elapses
    /// first.
    pub(crate) async fn stream_ticks<T>(
        &self,
        endpoint: &'static str,
        deadline: Option<std::time::Duration>,
        query: Result<Box<dyn EndpointQuery>, Error>,
        parse: TickParser<T>,
        callback: &mut ChunkCallback<'_, T>,
    ) -> Result<(), Error>
    where
        T: WireColumns + Send,
    {
        let deadline = effective_deadline(deadline, self.config().market_data.request_timeout_secs);
        run_with_optional_deadline(deadline, async move {
            tracing::debug!(endpoint, "gRPC streaming request");
            metrics::counter!("thetadatadx.grpc.requests", "endpoint" => endpoint).increment(1);
            let started = std::time::Instant::now();
            let query = query?;
            // Every band of a fan-out delivers through this one lock.
            // `Mutex<C>` where `C: Send` keeps the future Send (the Python
            // SDK's `spawn_awaitable` requires this).
            let callback = tokio::sync::Mutex::new(callback);
            let stopped = AtomicBool::new(false);
            let (callback, stopped) = (&callback, &stopped);
            match query.plan(self) {
                Some(plan) => {
                    let policy = &self.config().retry;
                    let mut bands: Vec<ShardStreamFuture<'_>> =
                        Vec::with_capacity(plan.bands.len());
                    for band in &plan.bands {
                        let query = query.band(band);
                        bands.push(Box::pin(tracing::Instrument::instrument(
                            async move {
                                let band_started = std::time::Instant::now();
                                // Per-band no-resume guard: a band that has
                                // handed rows to the callback must not replay
                                // from chunk zero, while a sibling's delivery
                                // leaves this band's retry budget intact.
                                let delivered = AtomicBool::new(false);
                                let outcome = async {
                                    // Each band takes its OWN permit; the
                                    // joining driver holds none.
                                    let _permit = self.acquire_request_permit().await?;
                                    run_streaming_retry_loop(
                                        self.session(),
                                        policy,
                                        endpoint,
                                        &delivered,
                                        |snap| {
                                            self.deliver_ticks(
                                                &*query, snap.uuid, parse, callback, &delivered,
                                                stopped,
                                            )
                                        },
                                    )
                                    .await
                                }
                                .await;
                                let delivered = delivered.load(Ordering::Relaxed);
                                tracing::debug!(
                                    delivered,
                                    ok = outcome.is_ok(),
                                    elapsed_ms = band_started.elapsed().as_millis() as u64,
                                    "shard band finished"
                                );
                                (delivered, outcome)
                            },
                            super::shard::band_span(band),
                        )));
                    }
                    super::shard::join_streaming_shards(&plan.bands, bands).await?;
                }
                None => {
                    let _permit = self.acquire_request_permit().await?;
                    let delivered = AtomicBool::new(false);
                    run_streaming_retry_loop(
                        self.session(),
                        &self.config().retry,
                        endpoint,
                        &delivered,
                        |snap| {
                            self.deliver_ticks(
                                &*query, snap.uuid, parse, callback, &delivered, stopped,
                            )
                        },
                    )
                    .await?;
                }
            }
            metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => endpoint)
                .record(started.elapsed().as_secs_f64() * 1_000.0);
            Ok(())
        })
        .await
    }

    /// One attempt of a streamed call: open `query`'s server stream, parse
    /// each chunk with `parse`, pair its rows with the columns its wire
    /// carried, each row's root included, and hand the [`Ticks`] to
    /// `callback` under its lock, marking `delivered` once a chunk with rows
    /// has reached it (the no-resume guard read by
    /// [`run_streaming_retry_loop`]).
    ///
    /// The lock is held across the callback's await, so concurrent bands of
    /// a fan-out never overlap callback runs. A `Break` from the callback
    /// stops this drain, and `stopped`, shared by every band of the call,
    /// keeps any band from opening a request or calling the callback again.
    /// The flag is read and set under the lock, so no call slips in after
    /// the one that returned `Break`. The first parse failure is terminal:
    /// it breaks the drain and surfaces after the stream is released.
    ///
    /// # Errors
    ///
    /// Propagates open, stream and decode errors, or the first parse failure.
    async fn deliver_ticks<T>(
        &self,
        query: &dyn EndpointQuery,
        uuid: String,
        parse: TickParser<T>,
        callback: &tokio::sync::Mutex<&mut ChunkCallback<'_, T>>,
        delivered: &AtomicBool,
        stopped: &AtomicBool,
    ) -> Result<(), Error>
    where
        T: WireColumns + Send,
    {
        // The callback ended the call: a band still waiting to start opens
        // no request.
        if stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        let lease = self.channel();
        let stream = query
            .open(&lease, self.build_query_info(uuid))
            .await
            .map_err(Error::from)?;
        let mut decode_error: Option<Error> = None;
        // Marks the callback's await as this client's delivery scope, so a
        // same-client request awaited inside the handler fails fast instead
        // of waiting on permits this pull holds (see
        // `acquire_request_permit`).
        let sem_addr = std::sync::Arc::as_ptr(&self.request_semaphore) as usize;
        let drain_result = self
            .for_each_chunk_async_control(stream, None, |headers, rows| {
                let mut stop_stream = decode_error.is_some();
                let ticks = if stop_stream {
                    None
                } else {
                    let chunk_table = proto::DataTable {
                        headers,
                        data_table: rows,
                    };
                    match parse(&chunk_table) {
                        Ok(rows) => {
                            let columns = chunk_columns::<T>(&chunk_table);
                            Some(Ticks::new(rows, columns))
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
                        // Only a chunk with rows marks the stream delivered:
                        // an empty chunk hands the callback nothing a replay
                        // could duplicate.
                        let delivered_nonempty = !ticks.is_empty();
                        let mut callback = callback.lock().await;
                        if stopped.load(Ordering::Relaxed) {
                            return ControlFlow::Break(());
                        }
                        let callback_fut = (*callback)(ticks);
                        if delivered_nonempty {
                            delivered.store(true, Ordering::Relaxed);
                        }
                        if super::client::in_delivery_scope(sem_addr, callback_fut)
                            .await
                            .is_break()
                        {
                            stopped.store(true, Ordering::Relaxed);
                            return ControlFlow::Break(());
                        }
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

    /// Send one generated endpoint's query and hand each chunk's column
    /// headers and cells to `handler` as the vendor sends them, unconverted:
    /// the body of every builder's `stream_tables`.
    ///
    /// The request goes upstream once as built, never split into bands, so
    /// the rows keep the vendor's order. `deadline` resolves as on every
    /// other call, and a `Break` from `handler` ends the call. Transient
    /// failures are retried only until the first rows reach `handler`.
    #[cfg(feature = "__internal")]
    pub(crate) async fn stream_tables<F, Fut>(
        &self,
        endpoint: &'static str,
        deadline: Option<std::time::Duration>,
        query: &dyn EndpointQuery,
        handler: F,
    ) -> Result<(), Error>
    where
        F: FnMut(Vec<String>, Vec<proto::DataValueList>) -> Fut,
        Fut: Future<Output = ControlFlow<()>>,
    {
        let deadline = effective_deadline(deadline, self.config().market_data.request_timeout_secs);
        run_with_optional_deadline(deadline, async {
            tracing::debug!(endpoint, "gRPC streaming request");
            metrics::counter!("thetadatadx.grpc.requests", "endpoint" => endpoint).increment(1);
            let started = std::time::Instant::now();
            let _permit = self.acquire_request_permit().await?;
            let handler = tokio::sync::Mutex::new(handler);
            let delivered = AtomicBool::new(false);
            let (handler, delivered) = (&handler, &delivered);
            run_streaming_retry_loop(
                self.session(),
                &self.config().retry,
                endpoint,
                delivered,
                |snap| async move {
                    let stream = {
                        let lease = self.channel();
                        query
                            .open(&lease, self.build_query_info(snap.uuid))
                            .await
                            .map_err(Error::from)?
                    };
                    self.for_each_chunk_async_control(stream, None, |headers, rows| async move {
                        if !rows.is_empty() {
                            delivered.store(true, Ordering::Release);
                        }
                        (handler.lock().await)(headers, rows).await
                    })
                    .await
                },
            )
            .await?;
            metrics::histogram!("thetadatadx.grpc.latency_ms", "endpoint" => endpoint)
                .record(started.elapsed().as_secs_f64() * 1_000.0);
            Ok(())
        })
        .await
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
    // empty DataTable. Callers that need to
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
/// starts from an empty band. `produced` is the band's cross-attempt
/// row flag (the buffered twin of the streaming `delivered` guard),
/// set as soon as any chunk parses to rows: on an error the
/// attempt-local rows are discarded, and the flag is what lets
/// `join_shards` refuse to fold a rows-then-`NotFound` band to empty.
///
/// # Errors
///
/// Same conditions as [`collect_stream_table`], plus the first `parser`
/// failure (typed decode error), which is terminal for the attempt.
pub(crate) async fn collect_stream_typed<T, E, P>(
    mut stream: ServerStreaming<proto::ResponseData>,
    parser: P,
    produced: &std::sync::atomic::AtomicBool,
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
        if !collect.rows.is_empty() {
            produced.store(true, std::sync::atomic::Ordering::Relaxed);
        }
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
        if let Some(idx) = self.root_idx
            && !chunk.data_table.is_empty()
        {
            self.root.observe_chunk(
                self.rows.len(),
                chunk
                    .data_table
                    .iter()
                    .map(|row| super::decode::extract::root_cell_text(row, idx)),
            );
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
    } else if let Some(first) = saved_headers.as_deref()
        && !table.headers.is_empty()
        && table.headers.as_slice() != first
    {
        return Err(decode::DecodeError::ChunkHeaderDrift {
            chunk_index,
            first: first.join(","),
            chunk: table.headers.join(","),
        }
        .into());
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
