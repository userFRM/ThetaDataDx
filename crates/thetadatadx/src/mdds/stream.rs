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

        while let Some(response) = stream.next().await {
            let response = response?;

            // Use original_size as a rough pre-allocation hint on the first chunk.
            // Each DataValueList row is ~64 bytes on average (header-dependent),
            // so original_size / 64 gives a reasonable row-count estimate.
            if all_rows.is_empty() && response.original_size > 0 {
                all_rows.reserve(usize::try_from(response.original_size).unwrap_or(0) / 64);
            }

            let table = decode_chunk(decoder.as_ref(), response).await?;
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
        while let Some(response) = stream.next().await {
            let response = response?;
            let table = decode_chunk(decoder.as_ref(), response).await?;
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
) -> Result<proto::DataTable, Error> {
    if let Some(handle) = decoder {
        // `submit` short-circuits when the pool has been poisoned by
        // a prior worker-thread panic — surface as a transport-level
        // failure so the retry layer can decide on rebuild instead
        // of hanging on a dead ring.
        let rx = handle.submit(response).map_err(|err| Error::Transport {
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
        decode::decode_data_table(&response)
    }
}
