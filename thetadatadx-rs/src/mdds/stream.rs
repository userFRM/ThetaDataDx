//! gRPC response-stream helpers on [`HistoricalClient`].
//!
//! MDDS RPCs are server-streaming: each call yields a
//! [`crate::grpc::ServerStreaming`] of `ResponseData` messages whose
//! payloads are zstd-compressed `DataTable` chunks. Two collection
//! strategies are provided:
//!
//! - [`collect_stream`](HistoricalClient::collect_stream) (crate-private) — drains
//!   the stream into a single merged `DataTable`. Used by the generated list
//!   and parsed endpoint macros where the caller expects a finite result.
//! - [`for_each_chunk`](HistoricalClient::for_each_chunk) (public) — streams each
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

use super::client::HistoricalClient;

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

impl HistoricalClient {
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
    /// // `HistoricalClient` to open a server-streaming gRPC channel — no
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
}

/// Decode one streamed `ResponseData` and apply the first-chunk header
/// contract shared by [`HistoricalClient::for_each_chunk`] and
/// [`HistoricalClient::for_each_chunk_async`]: record the first non-empty
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
