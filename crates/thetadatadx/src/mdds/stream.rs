//! gRPC response-stream helpers on [`MddsClient`].
//!
//! MDDS RPCs are server-streaming: each call yields a
//! [`crate::grpc::ServerStreaming`] of `ResponseData` messages whose
//! payloads are zstd-compressed `DataTable` chunks. Two collection
//! strategies are provided:
//!
//! - [`collect_stream`](MddsClient::collect_stream) (crate-private) — drains
//!   the stream into a single merged `DataTable`. Used by the generated list
//!   and parsed endpoint macros where the caller expects a finite result.
//! - [`for_each_chunk`](MddsClient::for_each_chunk) (public) — streams each
//!   chunk into a caller-supplied closure without materializing every row.
//!   Used by the generated streaming builders and public enough for callers
//!   processing multi-million-row responses.

use tokio_stream::StreamExt;

use crate::decode;
use crate::error::Error;
use crate::grpc::ServerStreaming;
use crate::proto;

use super::client::MddsClient;

impl MddsClient {
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
    pub(crate) async fn collect_stream(
        &self,
        mut stream: ServerStreaming<proto::ResponseData>,
    ) -> Result<proto::DataTable, Error> {
        let mut all_rows = Vec::new();
        let mut headers: Vec<String> = Vec::new();
        let mut chunk_index: usize = 0;

        // Clone the decoder handle (if any) once before the receive
        // loop so each chunk hands off without re-borrowing the
        // stream's `Option`. `None` means inline decode on this
        // task — used by the unit-test channels that construct a
        // `Channel` without a pool.
        let decoder = stream.decoder().cloned();
        let max_message_size = stream.max_message_size();

        while let Some(response) = stream.next().await {
            let response = response?;

            // Use original_size as a rough pre-allocation hint on the first chunk.
            // Each DataValueList row is ~64 bytes on average (header-dependent),
            // so original_size / 64 gives a reasonable row-count estimate.
            //
            // R1 bound: cap the hint at `max_message_size` so a hostile
            // `original_size = i32::MAX` cannot inflate `all_rows`'
            // capacity past the channel's configured ceiling before the
            // decompression layer rejects the payload.
            if all_rows.is_empty() && response.original_size > 0 {
                let hint = usize::try_from(response.original_size).unwrap_or(0);
                all_rows.reserve(hint.min(max_message_size) / 64);
            }

            let table = decode_chunk(decoder.as_ref(), response, max_message_size).await?;
            if headers.is_empty() {
                headers = table.headers;
            } else if !table.headers.is_empty() && table.headers != headers {
                // Mid-stream schema drift. The old accumulator silently kept
                // the first chunk's headers and piled rows under them; surface
                // the mismatch instead so downstream decoders do not read
                // columns under the wrong names.
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
    /// // `MddsClient` to open a server-streaming gRPC channel — no
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
        // non-empty headers disagree with the first-chunk schema: accepting
        // them silently would let the callback read columns under the wrong
        // names, which is the exact failure mode P13 asked to close.
        let mut saved_headers: Option<Vec<String>> = None;
        let mut chunk_index: usize = 0;
        let decoder = stream.decoder().cloned();
        let max_message_size = stream.max_message_size();
        while let Some(response) = stream.next().await {
            let response = response?;
            let table = decode_chunk(decoder.as_ref(), response, max_message_size).await?;
            if saved_headers.is_none() && !table.headers.is_empty() {
                saved_headers = Some(table.headers.clone());
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
}

/// Route a single `ResponseData` chunk through the channel's decoder
/// pool (when attached) so the zstd decompress + `DataTable` decode
/// runs on a dedicated thread instead of the tokio reactor. Falls
/// back to inline decode on the caller's task when no decoder is
/// attached — that path covers `Channel::connect_*` constructors
/// used by unit-test fixtures that do not need the pool overhead.
async fn decode_chunk(
    decoder: Option<&crate::grpc::DecoderHandle>,
    response: proto::ResponseData,
    max_message_size: usize,
) -> Result<proto::DataTable, Error> {
    if let Some(handle) = decoder {
        // `submit` short-circuits when the pool has been poisoned by
        // a prior worker-thread panic — surface as a transport-level
        // failure so the retry layer can decide on rebuild instead
        // of hanging on a dead ring.
        let rx = handle
            .submit(response, max_message_size)
            .map_err(|err| Error::Transport {
                kind: crate::error::TransportErrorKind::DecoderPoisoned,
                message: format!("mdds decoder pool rejected submission: {err}"),
            })?;
        match rx.await {
            Ok(result) => result,
            // `oneshot::Receiver` errors only when the sender is
            // dropped — which on our pool side means the consumer
            // thread was torn down mid-flight. Surface as Transport
            // so the retry layer can decide.
            Err(_) => Err(Error::Transport {
                kind: crate::error::TransportErrorKind::DecoderReplyDropped,
                message: "mdds decoder pool dropped its reply channel".to_string(),
            }),
        }
    } else {
        decode::decode_data_table_with_max(&response, max_message_size)
    }
}

#[cfg(test)]
mod streaming_decode_contract {
    //! Issue #565 contract pin: the chunk-by-chunk decode primitive that
    //! the generated `.stream(handler)` method on every parsed builder
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

    #[tokio::test]
    async fn decode_chunk_handles_inline_decode_with_no_pool() {
        // `decode_chunk(None, ...)` is the inline-decode branch taken
        // when no `DecoderHandle` is attached to the channel. Each
        // chunk's owned `ResponseData` is consumed by value, the
        // working buffer is freed before the next chunk is fetched,
        // and the returned `DataTable` carries exactly the rows the
        // chunk encoded. Pins the inline branch — the higher-level
        // `for_each_chunk` peak-memory contract is exercised by the
        // integration tests in `tests/`.
        let chunks = vec![
            make_chunk(&[("AAPL", 1), ("MSFT", 2)]),
            make_chunk(&[("GOOG", 3)]),
            make_chunk(&[("NVDA", 4), ("AMD", 5), ("INTC", 6)]),
        ];
        let mut per_chunk_row_counts = Vec::new();
        let mut total_rows = 0_usize;
        let max = 4 * 1024 * 1024;
        for chunk in chunks {
            let table = decode_chunk(None, chunk, max).await.expect("inline decode");
            per_chunk_row_counts.push(table.data_table.len());
            total_rows += table.data_table.len();
        }
        assert_eq!(per_chunk_row_counts, vec![2, 1, 3]);
        assert_eq!(total_rows, 6);
    }

    #[tokio::test]
    async fn max_message_size_ceiling_enforced_per_chunk() {
        // A hostile peer that sets `original_size = i32::MAX` on a
        // single chunk inside a streaming response cannot bypass the
        // ceiling — the per-chunk decode rejects it BEFORE allocation.
        // R1's `max_message_size` clamp applies on every chunk the
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
        let err = decode_chunk(None, hostile, 4 * 1024 * 1024)
            .await
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
