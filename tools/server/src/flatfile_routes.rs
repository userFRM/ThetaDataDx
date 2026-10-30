//! The flat-file routes: `/v3/{option,stock,index}/flat_file/{request}`.
//!
//! A flat file is the whole universe for one date. These routes answer the way
//! the terminal answers them: the file streams from the market-data service
//! chunk by chunk, and each chunk is written out in the terminal's layout as
//! soon as it arrives, by the same renderer the SDK's `<endpoint>_to_path`
//! calls write files with. Only the chunks in flight are held, whatever the
//! size of the file, except in `json_legacy`, which the terminal holds whole.
//!
//! The status line waits for the first chunk, so a refusal that arrives before
//! any data (no file for the date, a date outside the account's entitlement)
//! answers with the upstream's own status, as on every other route. A failure
//! after data has been sent aborts the body rather than ending it, so a client
//! cannot mistake a truncated file for a complete one.

use std::collections::HashMap;
use std::ops::ControlFlow;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Response;
use thetadatadx::endpoint::EndpointError;
use thetadatadx::flatfiles::{FileRenderer, FlatFileFormat};
use thetadatadx::wire::DataValueList;
use thetadatadx::{EndpointMeta, MarketDataClient};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;

use crate::handler::{self, BoundedQuery, MAX_QUERY_PARAMS, ResponseFormat};
use crate::state::AppState;

/// Rendered chunks held between the upstream and a slower client. One entry is
/// one gRPC chunk, so this bounds what a download holds in memory.
const CHUNKS_IN_FLIGHT: usize = 4;

type Chunk = Result<Bytes, std::io::Error>;

/// Serve the flat-file endpoint `ep` for one request.
pub(crate) async fn serve(
    State(state): State<AppState>,
    BoundedQuery(pairs): BoundedQuery<MAX_QUERY_PARAMS>,
    ep: &'static EndpointMeta,
) -> Response {
    respond(state, ep, handler::fold_query_pairs(ep, pairs)).await
}

/// Answer one flat-file request from its folded query parameters, streaming
/// the file from the service through `client`.
pub(crate) async fn respond<C>(
    client: C,
    ep: &'static EndpointMeta,
    params: HashMap<String, String>,
) -> Response
where
    C: AsRef<MarketDataClient> + Send + Sync + 'static,
{
    if let Some(response) = handler::deprecated_v2_param_response(&params) {
        return response;
    }
    let args = match handler::build_endpoint_args(ep, &params) {
        Ok(args) => args,
        Err(error) => return handler::endpoint_error_response(ep, error),
    };
    let format = match handler::parse_response_format(&params) {
        Ok(format) => format,
        Err(error) => return handler::endpoint_error_response(ep, error),
    };
    let date = match args.required_date("date") {
        Ok(date) => date.to_owned(),
        Err(error) => return handler::endpoint_error_response(ep, error),
    };

    let (chunks, body) = mpsc::channel(CHUNKS_IN_FLIGHT);
    let (started, start) = oneshot::channel();
    tokio::spawn(stream_file(client, ep, date, format, chunks, started));
    match start.await {
        Ok(Ok(())) => file_response(ep, &params, format, body),
        Ok(Err(error)) => handler::endpoint_error_response(ep, error),
        Err(_) => handler::plain_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the flat-file stream ended before it answered",
        ),
    }
}

fn file_response(
    ep: &EndpointMeta,
    params: &HashMap<String, String>,
    format: ResponseFormat,
    body: mpsc::Receiver<Chunk>,
) -> Response {
    let builder = Response::builder().status(StatusCode::OK);
    let builder = match format {
        ResponseFormat::Csv => builder
            .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}\"",
                    handler::csv_attachment_filename(ep, params)
                ),
            ),
        ResponseFormat::Json | ResponseFormat::JsonLegacy => {
            builder.header(header::CONTENT_TYPE, handler::JSON_CONTENT_TYPE)
        }
        ResponseFormat::Ndjson => {
            builder.header(header::CONTENT_TYPE, handler::NDJSON_CONTENT_TYPE)
        }
        ResponseFormat::Html => builder.header(header::CONTENT_TYPE, "text/html; charset=utf-8"),
    };
    builder
        .body(Body::from_stream(ReceiverStream::new(body)))
        .unwrap_or_else(|error| {
            handler::plain_error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())
        })
}

/// Stream the file from the service into `chunks`, reporting on `started`
/// whether the response can begin.
async fn stream_file(
    client: impl AsRef<MarketDataClient>,
    ep: &'static EndpointMeta,
    date: String,
    format: ResponseFormat,
    chunks: mpsc::Sender<Chunk>,
    started: oneshot::Sender<Result<(), EndpointError>>,
) {
    let client = client.as_ref();
    let mut file = FileWriter::new(format, chunks.clone(), started);
    // The download has no deadline. The client bounds it instead: the
    // download ends, and the upstream call is cancelled, as soon as the client
    // stops reading, so a live but silent upstream holds a request only while
    // its client waits.
    let download =
        client.stream_flat_file(ep.name, &date, |headers, rows| file.chunk(&headers, &rows));
    let outcome = tokio::select! {
        outcome = download => outcome,
        () = chunks.closed() => Ok(()),
    };
    file.finish(outcome).await;
}

/// Writes one file's chunks in the requested format.
struct FileWriter {
    renderer: FileRenderer,
    failed: bool,
    chunks: mpsc::Sender<Chunk>,
    started: Option<oneshot::Sender<Result<(), EndpointError>>>,
}

impl FileWriter {
    fn new(
        format: ResponseFormat,
        chunks: mpsc::Sender<Chunk>,
        started: oneshot::Sender<Result<(), EndpointError>>,
    ) -> Self {
        let format = match format {
            ResponseFormat::Csv => FlatFileFormat::Csv,
            ResponseFormat::Json => FlatFileFormat::Json,
            ResponseFormat::JsonLegacy => FlatFileFormat::JsonLegacy,
            ResponseFormat::Ndjson => FlatFileFormat::Jsonl,
            ResponseFormat::Html => FlatFileFormat::Html,
        };
        Self {
            renderer: FileRenderer::new(format),
            failed: false,
            chunks,
            started: Some(started),
        }
    }

    /// Render one chunk and hand it to the response body. Breaks the download
    /// once the client has gone, or on a cell the file cannot carry.
    fn chunk(
        &mut self,
        headers: &[String],
        rows: &[DataValueList],
    ) -> impl std::future::Future<Output = ControlFlow<()>> + Send + 'static + use<> {
        let started = self.started.take();
        let rendered = match self.renderer.chunk(headers, rows) {
            Ok(rendered) => {
                if let Some(started) = started {
                    let _ = started.send(Ok(()));
                }
                (!rendered.is_empty()).then(|| Ok(Bytes::from(rendered)))
            }
            Err(error) => {
                self.failed = true;
                tracing::warn!(%error, "flat-file chunk could not be written");
                match started {
                    // Nothing has been sent: the response carries the failure.
                    Some(started) => {
                        let _ = started.send(Err(EndpointError::Server(error)));
                        None
                    }
                    None => Some(Err(std::io::Error::other(error.to_string()))),
                }
            }
        };
        let failed = self.failed;
        let chunks = self.chunks.clone();
        async move {
            let sent = match rendered {
                Some(chunk) => chunks.send(chunk).await.is_ok(),
                None => true,
            };
            if sent && !failed {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        }
    }

    /// Close the file once the download ends: write what remains of it, or
    /// report the failure the way the response can still carry it.
    async fn finish(mut self, outcome: Result<(), thetadatadx::Error>) {
        if self.failed {
            // `chunk` already reported the failure.
            return;
        }
        match outcome {
            Ok(()) => {
                // Each piece goes out as its own body chunk, so a
                // `json_legacy` file is never copied whole.
                let tail = self.renderer.finish();
                for piece in tail.into_iter().filter(|piece| !piece.is_empty()) {
                    if self.chunks.send(Ok(Bytes::from(piece))).await.is_err() {
                        break;
                    }
                }
                if let Some(started) = self.started.take() {
                    let _ = started.send(Ok(()));
                }
            }
            Err(error) => match self.started.take() {
                Some(started) => {
                    let _ = started.send(Err(EndpointError::Server(error)));
                }
                None => {
                    tracing::warn!(%error, "flat-file download failed after the response began");
                    let _ = self
                        .chunks
                        .send(Err(std::io::Error::other(error.to_string())))
                        .await;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use thetadatadx::wire::{DataValue, data_value::DataType};

    use super::*;

    /// A chunk shaped like the service's: the header line, then rows whose
    /// quoted cells are text (the service sends prices as decimal text) and
    /// whose bare cells are integers.
    fn chunk(file: &str) -> (Vec<String>, Vec<DataValueList>) {
        let cell = |cell: &str| DataValue {
            data_type: Some(
                match cell.strip_prefix('"').and_then(|c| c.strip_suffix('"')) {
                    Some(text) => DataType::Text(text.to_owned()),
                    None => DataType::Number(cell.parse().expect("integer cell")),
                },
            ),
        };
        let mut lines = file.lines();
        let headers = lines
            .next()
            .expect("header line")
            .split(',')
            .map(str::to_owned)
            .collect();
        let rows = lines
            .map(|line| DataValueList {
                values: line.split(',').map(cell).collect(),
            })
            .collect();
        (headers, rows)
    }

    /// A download that fails before any data answers with the failure's own
    /// status; one that fails after data has gone out ends the body in an
    /// error, so the client sees a broken transfer rather than a short file.
    #[tokio::test]
    async fn a_failed_download_is_never_answered_as_a_complete_file() {
        let failure = || thetadatadx::Error::config_internal("upstream reset");

        let (chunks, mut body) = mpsc::channel(4);
        let (started, start) = oneshot::channel();
        FileWriter::new(ResponseFormat::Csv, chunks, started)
            .finish(Err(failure()))
            .await;
        assert!(matches!(start.await, Ok(Err(EndpointError::Server(_)))));
        assert!(body.recv().await.is_none(), "nothing reaches the body");

        let (chunks, mut body) = mpsc::channel(4);
        let (started, start) = oneshot::channel();
        let mut file = FileWriter::new(ResponseFormat::Csv, chunks, started);
        let (headers, rows) = chunk("root,date\n\"HCMA\",20261021\n");
        assert!(file.chunk(&headers, &rows).await.is_continue());
        file.finish(Err(failure())).await;
        assert!(matches!(start.await, Ok(Ok(()))));
        assert!(matches!(body.recv().await, Some(Ok(_))));
        assert!(
            matches!(body.recv().await, Some(Err(_))),
            "the body ends in an error"
        );

        // A cell the file cannot carry stops the download the same way: as
        // the response's status before any data, as a body error after it.
        let bad = DataValueList {
            values: vec![DataValue {
                data_type: Some(DataType::Price(thetadatadx::wire::Price {
                    value: 1,
                    r#type: 10,
                })),
            }],
        };
        let headers = vec!["bid".to_owned()];
        let (chunks, mut body) = mpsc::channel(4);
        let (started, start) = oneshot::channel();
        let mut file = FileWriter::new(ResponseFormat::Csv, chunks, started);
        assert!(
            file.chunk(&headers, std::slice::from_ref(&bad))
                .await
                .is_break()
        );
        file.finish(Ok(())).await;
        assert!(matches!(start.await, Ok(Err(EndpointError::Server(_)))));
        assert!(body.recv().await.is_none(), "nothing reaches the body");

        let (chunks, mut body) = mpsc::channel(4);
        let (started, start) = oneshot::channel();
        let mut file = FileWriter::new(ResponseFormat::Csv, chunks, started);
        let (good_headers, rows) = chunk("root,date\n\"HCMA\",20261021\n");
        assert!(file.chunk(&good_headers, &rows).await.is_continue());
        assert!(file.chunk(&headers, &[bad]).await.is_break());
        file.finish(Ok(())).await;
        assert!(matches!(start.await, Ok(Ok(()))));
        assert!(matches!(body.recv().await, Some(Ok(_))));
        assert!(
            matches!(body.recv().await, Some(Err(_))),
            "the body ends in an error"
        );
        assert!(body.recv().await.is_none(), "nothing follows the error");
    }
}
