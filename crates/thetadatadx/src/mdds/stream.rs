//! gRPC response-stream helpers on [`MddsClient`].
//!
//! MDDS RPCs are server-streaming: each call yields a `tonic::Streaming` of
//! `ResponseData` messages whose payloads are zstd-compressed `DataTable`
//! chunks. Two collection strategies are provided:
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
        mut stream: tonic::Streaming<proto::ResponseData>,
    ) -> Result<proto::DataTable, Error> {
        let mut all_rows = Vec::new();
        let mut headers = Vec::new();

        while let Some(response) = stream.next().await {
            let response = response?;

            // Use original_size as a rough pre-allocation hint on the first chunk.
            // Each DataValueList row is ~64 bytes on average (header-dependent),
            // so original_size / 64 gives a reasonable row-count estimate.
            if all_rows.is_empty() && response.original_size > 0 {
                all_rows.reserve(usize::try_from(response.original_size).unwrap_or(0) / 64);
            }

            let table = decode::decode_data_table(&response)?;
            if headers.is_empty() {
                headers = table.headers;
            }
            all_rows.extend(table.data_table);
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
    /// ```text
    /// let request = /* build your gRPC request */;
    /// let stream = client.stub().get_stock_history_trade(request).await?.into_inner();
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
        mut stream: tonic::Streaming<proto::ResponseData>,
        mut f: F,
    ) -> Result<(), Error>
    where
        F: FnMut(&[String], &[proto::DataValueList]),
    {
        // Preserve first-chunk headers across all chunks, matching collect_stream behavior.
        let mut saved_headers: Option<Vec<String>> = None;
        while let Some(response) = stream.next().await {
            let response = response?;
            let table = decode::decode_data_table(&response)?;
            if saved_headers.is_none() && !table.headers.is_empty() {
                saved_headers = Some(table.headers.clone());
            }
            let headers = if table.headers.is_empty() {
                saved_headers.as_deref().unwrap_or(&[])
            } else {
                &table.headers
            };
            f(headers, &table.data_table);
        }
        Ok(())
    }
}
