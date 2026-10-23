//! The flat-file routes: `/v3/{option,stock,index}/flat_file/{request}`.
//!
//! A flat file is the whole universe for one date. These routes answer the way
//! the terminal answers them: the file streams from the market-data service
//! chunk by chunk, and each chunk is written out as soon as it arrives, under
//! the column names the service sends and with each cell as the service sent
//! it: text (the root, the right, and every price, which the service sends as
//! decimal text) quoted, integers bare. Only the chunks in flight are held,
//! whatever the size of the file, except in `json_legacy`: its shape is one
//! array per column, so no column can close before the last row arrives, and
//! the file is held until then, as the terminal holds it.
//!
//! The status line waits for the first chunk, so a refusal that arrives before
//! any data (no file for the date, a date outside the account's entitlement)
//! answers with the upstream's own status, as on every other route. A failure
//! after data has been sent aborts the body rather than ending it, so a client
//! cannot mistake a truncated file for a complete one.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::ControlFlow;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Response;
use thetadatadx::EndpointMeta;
use thetadatadx::endpoint::EndpointError;
use thetadatadx::wire::{DataValue, DataValueList, data_value::DataType};
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
    let params = handler::fold_query_pairs(ep, pairs);
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
    tokio::spawn(stream_file(state, ep, date, format, chunks, started));
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
    state: AppState,
    ep: &'static EndpointMeta,
    date: String,
    format: ResponseFormat,
    chunks: mpsc::Sender<Chunk>,
    started: oneshot::Sender<Result<(), EndpointError>>,
) {
    let client = state.client().market_data();
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
    format: ResponseFormat,
    wrote_header: bool,
    wrote_rows: bool,
    failed: bool,
    /// `json_legacy` only: each column so far, from its key to its last cell.
    columns: Vec<String>,
    chunks: mpsc::Sender<Chunk>,
    started: Option<oneshot::Sender<Result<(), EndpointError>>>,
}

impl FileWriter {
    fn new(
        format: ResponseFormat,
        chunks: mpsc::Sender<Chunk>,
        started: oneshot::Sender<Result<(), EndpointError>>,
    ) -> Self {
        Self {
            format,
            wrote_header: false,
            wrote_rows: false,
            failed: false,
            columns: Vec::new(),
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
        let rendered = match self.render(headers, rows) {
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
                        let _ = started.send(Err(EndpointError::Server(
                            thetadatadx::Error::config_internal(error.to_string()),
                        )));
                        None
                    }
                    None => Some(Err(error)),
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

    fn render(
        &mut self,
        headers: &[String],
        rows: &[DataValueList],
    ) -> Result<String, std::io::Error> {
        if self.format == ResponseFormat::JsonLegacy {
            if !self.wrote_header {
                self.columns = headers
                    .iter()
                    .enumerate()
                    .map(|(i, header)| {
                        let mut column = String::from(if i == 0 { "{" } else { "," });
                        push_json_string(&mut column, header);
                        column.push_str(":[");
                        column
                    })
                    .collect();
                self.wrote_header = true;
            }
            for cells in rows {
                for (i, column) in self.columns.iter_mut().enumerate() {
                    if self.wrote_rows {
                        column.push(',');
                    }
                    push_json_cell(column, cells.values.get(i))?;
                }
                self.wrote_rows = true;
            }
            return Ok(String::new());
        }
        let mut out = String::with_capacity(rows.len() * 160);
        if !self.wrote_header {
            prefix(&mut out, self.format, headers);
            self.wrote_header = true;
        }
        for cells in rows {
            row(
                &mut out,
                self.format,
                !self.wrote_rows,
                headers,
                &cells.values,
            )?;
            self.wrote_rows = true;
        }
        Ok(out)
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
                let mut tail = Vec::new();
                if self.format == ResponseFormat::JsonLegacy {
                    // Each column goes out as its own body chunk, so the file
                    // is never copied whole.
                    tail = std::mem::take(&mut self.columns);
                    for column in &mut tail {
                        column.push(']');
                    }
                    match tail.last_mut() {
                        Some(last) => last.push('}'),
                        None => tail.push("{}".to_owned()),
                    }
                } else {
                    let mut out = String::new();
                    if self.started.is_some() {
                        // No chunk arrived: the file is empty.
                        prefix(&mut out, self.format, &[]);
                    }
                    suffix(&mut out, self.format);
                    tail.push(out);
                }
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

fn prefix(out: &mut String, format: ResponseFormat, headers: &[String]) {
    match format {
        ResponseFormat::Csv => {
            out.push_str(&headers.join(","));
            out.push('\n');
        }
        ResponseFormat::Json => out.push_str("{\"response\":["),
        ResponseFormat::Ndjson | ResponseFormat::JsonLegacy => {}
        ResponseFormat::Html => {
            out.push_str(crate::format::HTML_START);
            out.push_str("<thead><tr>");
            for header in headers {
                let _ = write!(out, "<th>{}</th>", crate::format::escape_html(header));
            }
            out.push_str("</tr></thead>\n<tbody>");
        }
    }
}

/// The terminal ends a CSV file with one blank line.
fn suffix(out: &mut String, format: ResponseFormat) {
    match format {
        ResponseFormat::Csv => out.push('\n'),
        ResponseFormat::Ndjson | ResponseFormat::JsonLegacy => {}
        ResponseFormat::Json => out.push_str("]}"),
        ResponseFormat::Html => out.push_str(crate::format::HTML_END),
    }
}

/// One cell as the service sent it. The flat files carry text and integers;
/// any other cell type ends the file in an error rather than being written in
/// a form the terminal never writes.
fn cell(value: &DataValue) -> Result<Option<Cell<'_>>, std::io::Error> {
    match value.data_type.as_ref() {
        Some(DataType::Text(text)) => Ok(Some(Cell::Text(text))),
        Some(DataType::Number(n)) => Ok(Some(Cell::Int(*n))),
        Some(DataType::NullValue(_)) => Ok(None),
        other => Err(std::io::Error::other(format!(
            "flat-file cell of an unexpected type: {other:?}"
        ))),
    }
}

enum Cell<'a> {
    Text(&'a str),
    Int(i64),
}

fn row(
    out: &mut String,
    format: ResponseFormat,
    first: bool,
    headers: &[String],
    values: &[DataValue],
) -> Result<(), std::io::Error> {
    match format {
        ResponseFormat::Csv => {
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match cell(value)? {
                    Some(Cell::Text(text)) => {
                        out.push('"');
                        out.push_str(&text.replace('"', "\"\""));
                        out.push('"');
                    }
                    Some(Cell::Int(n)) => {
                        let _ = write!(out, "{n}");
                    }
                    None => {}
                }
            }
            out.push('\n');
        }
        ResponseFormat::Json | ResponseFormat::Ndjson => {
            if format == ResponseFormat::Json && !first {
                out.push(',');
            }
            out.push('{');
            for (i, (header, value)) in headers.iter().zip(values).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_string(out, header);
                out.push(':');
                push_json_cell(out, Some(value))?;
            }
            out.push('}');
            if format == ResponseFormat::Ndjson {
                out.push('\n');
            }
        }
        ResponseFormat::Html => {
            out.push_str("<tr>");
            for value in values {
                out.push_str("<td>");
                match cell(value)? {
                    Some(Cell::Text(text)) => out.push_str(&crate::format::escape_html(text)),
                    Some(Cell::Int(n)) => {
                        let _ = write!(out, "{n}");
                    }
                    None => out.push_str("null"),
                }
                out.push_str("</td>");
            }
            out.push_str("</tr>\n");
        }
        // Written column by column in `FileWriter::render`.
        ResponseFormat::JsonLegacy => {}
    }
    Ok(())
}

/// One cell as JSON: text as a string, an integer as a number, and a missing
/// or null cell as `null`.
fn push_json_cell(out: &mut String, value: Option<&DataValue>) -> Result<(), std::io::Error> {
    match value.map(cell).transpose()?.flatten() {
        Some(Cell::Text(text)) => push_json_string(out, text),
        Some(Cell::Int(n)) => {
            let _ = write!(out, "{n}");
        }
        None => out.push_str("null"),
    }
    Ok(())
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
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

    async fn written(format: ResponseFormat, file: &str) -> String {
        let (chunks, mut body) = mpsc::channel(64);
        let (started, _start) = oneshot::channel();
        let mut writer = FileWriter::new(format, chunks, started);
        let (headers, rows) = chunk(file);
        assert!(writer.chunk(&headers, &rows).await.is_continue());
        writer.finish(Ok(())).await;
        let mut out = Vec::new();
        while let Some(piece) = body.recv().await {
            out.extend_from_slice(&piece.expect("the file is written whole"));
        }
        String::from_utf8(out).expect("the file is UTF-8")
    }

    /// A flat file is written as the terminal writes it: the terminal's own
    /// rows for 2026-10-21, sent through as the service's cells, come back
    /// byte for byte, the sub-penny `"0.00010"` and the closing blank line
    /// included.
    #[tokio::test]
    async fn flat_files_are_written_as_the_terminal_writes_them() {
        let stock_eod = "root,ms_of_day,ms_of_day2,open,high,low,close,volume,count,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"HCMA\",62128177,57600350,\"10.3\",\"10.3\",\"10.3\",\"10.3\",112241,101,100,7,\"10.26\",0,100,7,\"10.35\",0,20261021\n\
            \"HCMC\",62128177,57577936,\"0.00010\",\"0.00010\",\"0.0\",\"0.00010\",42627531,109,0,0,\"0.0\",0,0,0,\"0.0\",0,20261021\n";
        assert_eq!(
            written(ResponseFormat::Csv, stock_eod).await,
            format!("{stock_eod}\n")
        );

        let option_quote = "root,expiration,strike,right,interval_ms_of_day,quote_ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"ABG\",20270115,\"115.0\",\"C\",34260000,34259636,1,11,\"61.0\",50,1,11,\"65.0\",50,20261021\n";
        assert_eq!(
            written(ResponseFormat::Csv, option_quote).await,
            format!("{option_quote}\n")
        );

        // The JSON form carries the same cells: text as strings, integers as
        // numbers, in the `response` envelope.
        assert_eq!(
            written(
                ResponseFormat::Json,
                "root,bid,date\n\"HCMC\",\"0.00010\",20261021\n"
            )
            .await,
            "{\"response\":[{\"root\":\"HCMC\",\"bid\":\"0.00010\",\"date\":20261021}]}"
        );

        // The legacy JSON form is one array per column, the same cells in
        // the same order, row `i` at index `i` of every array.
        assert_eq!(
            written(
                ResponseFormat::JsonLegacy,
                "root,bid,date\n\"HCMA\",\"10.26\",20261021\n\"HCMC\",\"0.00010\",20261021\n"
            )
            .await,
            "{\"root\":[\"HCMA\",\"HCMC\"],\"bid\":[\"10.26\",\"0.00010\"],\"date\":[20261021,20261021]}"
        );

        // The HTML form is the terminal's page, as the regular routes write it.
        assert_eq!(
            written(
                ResponseFormat::Html,
                "root,bid,date\n\"HCMC\",\"0.00010\",20261021\n"
            )
            .await,
            "<!DOCTYPE html><html lang=\"en\">\n<head><title>Theta Data</title></head>\n<body>\n\
             <table style=\"width:100%; text-align: center;\"><thead><tr><th>root</th><th>bid</th><th>date</th></tr></thead>\n\
             <tbody><tr><td>HCMC</td><td>0.00010</td><td>20261021</td></tr>\n\
             </tbody></table></body></html>\n"
        );
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
