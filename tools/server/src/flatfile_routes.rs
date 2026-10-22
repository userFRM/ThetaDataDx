//! The flat-file routes: `/v3/{option,stock,index}/flat_file/{request}`.
//!
//! A flat file is the whole universe for one date. These routes answer the way
//! the terminal answers them: the registry endpoint streams the file from the
//! market-data service chunk by chunk, and each chunk is written out as soon
//! as it is decoded, under the terminal's column names (`root`, `ms_of_day`,
//! `ms_of_day2`, `interval_ms_of_day`, ...) with text and prices quoted and
//! integers bare. Only the chunks in flight are held, whatever the size of the
//! file.
//!
//! The status line waits for the first chunk, so a refusal that arrives before
//! any data (no file for the date, a date outside the account's entitlement)
//! answers with the upstream's own status, as on every other route. A failure
//! after data has been sent aborts the body rather than ending it, so a client
//! cannot mistake a truncated file for a complete one.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::ControlFlow;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use thetadatadx::endpoint::EndpointError;
use thetadatadx::flatfiles::FlatFileFormat;
use thetadatadx::{
    ColumnPresence, EndpointMeta, EodTick, IntervalQuoteTick, OpenInterestTick, Ticks,
    TradeQuoteTick,
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;

use crate::handler::{self, BoundedQuery, MAX_QUERY_PARAMS};
use crate::state::AppState;

/// Rendered chunks held between the upstream and a slower client. One entry is
/// one decoded gRPC chunk, so this bounds what a download holds in memory.
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
    let format = match params
        .get("format")
        .map_or(Ok(FlatFileFormat::Csv), |f| f.parse())
    {
        Ok(format) => format,
        Err(message) => return handler::plain_error_response(StatusCode::BAD_REQUEST, &message),
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
    format: FlatFileFormat,
    body: mpsc::Receiver<Chunk>,
) -> Response {
    let builder = Response::builder().status(StatusCode::OK);
    let builder = match format {
        FlatFileFormat::Csv => builder
            .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}\"",
                    handler::csv_attachment_filename(ep, params)
                ),
            ),
        FlatFileFormat::Json => builder.header(header::CONTENT_TYPE, handler::JSON_CONTENT_TYPE),
        FlatFileFormat::Jsonl => builder.header(header::CONTENT_TYPE, handler::NDJSON_CONTENT_TYPE),
        FlatFileFormat::Html => builder.header(header::CONTENT_TYPE, "text/html; charset=utf-8"),
    };
    builder
        .body(Body::from_stream(ReceiverStream::new(body)))
        .unwrap_or_else(|error| {
            handler::plain_error_response(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string())
        })
}

/// Stream the file from the endpoint into `chunks`, reporting on `started`
/// whether the response can begin.
async fn stream_file(
    state: AppState,
    ep: &'static EndpointMeta,
    date: String,
    format: FlatFileFormat,
    chunks: mpsc::Sender<Chunk>,
    started: oneshot::Sender<Result<(), EndpointError>>,
) {
    let client = state.client().market_data();
    let option = ep.category == "option";
    // A whole-universe file takes as long as it takes, so the download runs
    // with no deadline. The client bounds it instead: the download ends, and
    // the upstream call is cancelled, as soon as the client stops reading, so
    // a live but silent upstream holds a request only while its client waits.
    macro_rules! stream {
        ($tick:ty, $endpoint:ident) => {{
            let mut file = FileWriter::<$tick>::new(format, option, chunks.clone(), started);
            let download = client
                .$endpoint(&date)
                .with_deadline(Duration::ZERO)
                .stream_ticks_async(|ticks| file.chunk(&ticks));
            let outcome = tokio::select! {
                outcome = download => outcome,
                () = chunks.closed() => Ok(()),
            };
            file.finish(outcome).await;
        }};
    }
    match ep.name {
        "option_flat_file_eod" => stream!(EodTick, option_flat_file_eod),
        "option_flat_file_trade_quote" => stream!(TradeQuoteTick, option_flat_file_trade_quote),
        "option_flat_file_open_interest" => {
            stream!(OpenInterestTick, option_flat_file_open_interest)
        }
        "option_flat_file_quote" => stream!(IntervalQuoteTick, option_flat_file_quote),
        "stock_flat_file_eod" => stream!(EodTick, stock_flat_file_eod),
        "stock_flat_file_trade_quote" => stream!(TradeQuoteTick, stock_flat_file_trade_quote),
        "stock_flat_file_quote" => stream!(IntervalQuoteTick, stock_flat_file_quote),
        "index_flat_file_eod" => stream!(EodTick, index_flat_file_eod),
        other => {
            let _ = started.send(Err(EndpointError::Server(
                thetadatadx::Error::config_internal(format!(
                    "the server has no flat-file stream for {other}"
                )),
            )));
        }
    }
}

/// One cell of a flat-file row, rendered the way the terminal writes it.
#[derive(Clone, Copy)]
enum Value<'a> {
    /// Text, quoted: the root.
    Text(&'a str),
    /// A one-character code, quoted: the option right.
    Char(char),
    /// A bare integer.
    Int(i64),
    /// A price, quoted, with at least one decimal digit (`"808.0"`).
    Price(f64),
    /// A cell the vendor left empty.
    Null,
}

/// A column the terminal writes after the contract identity: its header, the
/// schema field the presence set names it by, and its value on a row.
struct Column<T> {
    header: &'static str,
    field: &'static str,
    value: fn(&T) -> Value<'static>,
}

const fn col<T>(
    header: &'static str,
    field: &'static str,
    value: fn(&T) -> Value<'static>,
) -> Column<T> {
    Column {
        header,
        field,
        value,
    }
}

fn code(value: i32, present: bool) -> Value<'static> {
    if present {
        Value::Int(value.into())
    } else {
        Value::Null
    }
}

/// A tick type a flat file decodes into, with the columns the terminal writes
/// for it, in the terminal's order.
trait FlatFileTick: Sized + Send + Sync + 'static {
    const COLUMNS: &'static [Column<Self>];

    /// Expiration, strike and right: the contract identity option files carry.
    fn contract(&self) -> (i32, f64, char);
}

impl FlatFileTick for EodTick {
    const COLUMNS: &'static [Column<Self>] = &[
        col("ms_of_day", "created_ms_of_day", |t| {
            Value::Int(t.created_ms_of_day.into())
        }),
        col("ms_of_day2", "last_trade_ms_of_day", |t| {
            Value::Int(t.last_trade_ms_of_day.into())
        }),
        col("open", "open", |t| Value::Price(t.open)),
        col("high", "high", |t| Value::Price(t.high)),
        col("low", "low", |t| Value::Price(t.low)),
        col("close", "close", |t| Value::Price(t.close)),
        col("volume", "volume", |t| Value::Int(t.volume)),
        col("count", "count", |t| Value::Int(t.count)),
        col("bid_size", "bid_size", |t| Value::Int(t.bid_size.into())),
        col("bid_exchange", "bid_exchange", |t| {
            code(t.bid_exchange, t.has_bid_exchange)
        }),
        col("bid", "bid", |t| Value::Price(t.bid)),
        col("bid_condition", "bid_condition", |t| {
            code(t.bid_condition, t.has_bid_condition)
        }),
        col("ask_size", "ask_size", |t| Value::Int(t.ask_size.into())),
        col("ask_exchange", "ask_exchange", |t| {
            code(t.ask_exchange, t.has_ask_exchange)
        }),
        col("ask", "ask", |t| Value::Price(t.ask)),
        col("ask_condition", "ask_condition", |t| {
            code(t.ask_condition, t.has_ask_condition)
        }),
        col("date", "date", |t| Value::Int(t.date.into())),
    ];

    fn contract(&self) -> (i32, f64, char) {
        (self.expiration, self.strike, self.right)
    }
}

impl FlatFileTick for TradeQuoteTick {
    const COLUMNS: &'static [Column<Self>] = &[
        col("ms_of_day", "ms_of_day", |t| Value::Int(t.ms_of_day.into())),
        col("sequence", "sequence", |t| Value::Int(t.sequence.into())),
        col("ext_condition1", "ext_condition1", |t| {
            code(t.ext_condition1, t.has_ext_condition1)
        }),
        col("ext_condition2", "ext_condition2", |t| {
            code(t.ext_condition2, t.has_ext_condition2)
        }),
        col("ext_condition3", "ext_condition3", |t| {
            code(t.ext_condition3, t.has_ext_condition3)
        }),
        col("ext_condition4", "ext_condition4", |t| {
            code(t.ext_condition4, t.has_ext_condition4)
        }),
        col("condition", "condition", |t| {
            code(t.condition, t.has_condition)
        }),
        col("size", "size", |t| Value::Int(t.size.into())),
        col("exchange", "exchange", |t| code(t.exchange, t.has_exchange)),
        col("price", "price", |t| Value::Price(t.price)),
        col("condition_flags", "condition_flags", |t| {
            Value::Int(t.condition_flags.into())
        }),
        col("price_flags", "price_flags", |t| {
            Value::Int(t.price_flags.into())
        }),
        col("volume_type", "volume_type", |t| {
            Value::Int(t.volume_type.into())
        }),
        col("records_back", "records_back", |t| {
            Value::Int(t.records_back.into())
        }),
        col("ms_of_day2", "quote_ms_of_day", |t| {
            Value::Int(t.quote_ms_of_day.into())
        }),
        col("bid_size", "bid_size", |t| Value::Int(t.bid_size.into())),
        col("bid_exchange", "bid_exchange", |t| {
            code(t.bid_exchange, t.has_bid_exchange)
        }),
        col("bid", "bid", |t| Value::Price(t.bid)),
        col("bid_condition", "bid_condition", |t| {
            code(t.bid_condition, t.has_bid_condition)
        }),
        col("ask_size", "ask_size", |t| Value::Int(t.ask_size.into())),
        col("ask_exchange", "ask_exchange", |t| {
            code(t.ask_exchange, t.has_ask_exchange)
        }),
        col("ask", "ask", |t| Value::Price(t.ask)),
        col("ask_condition", "ask_condition", |t| {
            code(t.ask_condition, t.has_ask_condition)
        }),
        col("date", "date", |t| Value::Int(t.date.into())),
    ];

    fn contract(&self) -> (i32, f64, char) {
        (self.expiration, self.strike, self.right)
    }
}

impl FlatFileTick for OpenInterestTick {
    const COLUMNS: &'static [Column<Self>] = &[
        col("ms_of_day", "ms_of_day", |t| Value::Int(t.ms_of_day.into())),
        col("open_interest", "open_interest", |t| {
            Value::Int(t.open_interest.into())
        }),
        col("date", "date", |t| Value::Int(t.date.into())),
    ];

    fn contract(&self) -> (i32, f64, char) {
        (self.expiration, self.strike, self.right)
    }
}

impl FlatFileTick for IntervalQuoteTick {
    const COLUMNS: &'static [Column<Self>] = &[
        col("interval_ms_of_day", "interval_ms_of_day", |t| {
            Value::Int(t.interval_ms_of_day.into())
        }),
        col("quote_ms_of_day", "quote_ms_of_day", |t| {
            Value::Int(t.quote_ms_of_day.into())
        }),
        col("bid_size", "bid_size", |t| Value::Int(t.bid_size.into())),
        col("bid_exchange", "bid_exchange", |t| {
            code(t.bid_exchange, t.has_bid_exchange)
        }),
        col("bid", "bid", |t| Value::Price(t.bid)),
        col("bid_condition", "bid_condition", |t| {
            code(t.bid_condition, t.has_bid_condition)
        }),
        col("ask_size", "ask_size", |t| Value::Int(t.ask_size.into())),
        col("ask_exchange", "ask_exchange", |t| {
            code(t.ask_exchange, t.has_ask_exchange)
        }),
        col("ask", "ask", |t| Value::Price(t.ask)),
        col("ask_condition", "ask_condition", |t| {
            code(t.ask_condition, t.has_ask_condition)
        }),
        col("date", "date", |t| Value::Int(t.date.into())),
    ];

    fn contract(&self) -> (i32, f64, char) {
        (self.expiration, self.strike, self.right)
    }
}

/// The columns one file is written with, fixed by its first chunk: the ones
/// the response carried, in the terminal's order.
struct Layout<T: 'static> {
    root: bool,
    contract: bool,
    columns: Vec<&'static Column<T>>,
}

impl<T: FlatFileTick> Layout<T> {
    /// The layout of a response whose columns are `presence`; every column
    /// when no chunk arrived to say which.
    fn new(option: bool, presence: Option<&ColumnPresence>) -> Self {
        let carried = |field: &str| presence.is_none_or(|p| p.contains(field));
        Self {
            root: presence.is_none_or(|p| p.symbol().is_some() || p.symbols().is_some()),
            contract: presence.map_or(option, |p| p.contains("expiration")),
            columns: T::COLUMNS.iter().filter(|c| carried(c.field)).collect(),
        }
    }

    fn headers(&self) -> impl Iterator<Item = &'static str> + '_ {
        let root = self.root.then_some("root");
        let contract = self
            .contract
            .then_some(["expiration", "strike", "right"])
            .into_iter()
            .flatten();
        root.into_iter()
            .chain(contract)
            .chain(self.columns.iter().map(|c| c.header))
    }

    fn values<'a>(
        &'a self,
        root: Option<&'a str>,
        tick: &'a T,
    ) -> impl Iterator<Item = (&'static str, Value<'a>)> + 'a {
        let root = self
            .root
            .then(|| ("root", root.map_or(Value::Null, Value::Text)));
        let contract = self
            .contract
            .then(|| {
                let (expiration, strike, right) = tick.contract();
                [
                    ("expiration", Value::Int(expiration.into())),
                    ("strike", Value::Price(strike)),
                    ("right", Value::Char(right)),
                ]
            })
            .into_iter()
            .flatten();
        root.into_iter().chain(contract).chain(
            self.columns
                .iter()
                .map(move |c| (c.header, (c.value)(tick))),
        )
    }
}

/// Writes one file's chunks in the requested format.
struct FileWriter<T: FlatFileTick> {
    format: FlatFileFormat,
    option: bool,
    layout: Option<Layout<T>>,
    wrote_rows: bool,
    chunks: mpsc::Sender<Chunk>,
    started: Option<oneshot::Sender<Result<(), EndpointError>>>,
}

impl<T: FlatFileTick> FileWriter<T> {
    fn new(
        format: FlatFileFormat,
        option: bool,
        chunks: mpsc::Sender<Chunk>,
        started: oneshot::Sender<Result<(), EndpointError>>,
    ) -> Self {
        Self {
            format,
            option,
            layout: None,
            wrote_rows: false,
            chunks,
            started: Some(started),
        }
    }

    /// Render one decoded chunk and hand it to the response body. Breaks the
    /// download once the client has gone.
    fn chunk(
        &mut self,
        ticks: &Ticks<T>,
    ) -> impl std::future::Future<Output = ControlFlow<()>> + Send + 'static {
        let rendered = self.render(ticks);
        if let Some(started) = self.started.take() {
            let _ = started.send(Ok(()));
        }
        let chunks = self.chunks.clone();
        async move {
            match chunks.send(Ok(Bytes::from(rendered))).await {
                Ok(()) => ControlFlow::Continue(()),
                Err(_) => ControlFlow::Break(()),
            }
        }
    }

    fn render(&mut self, ticks: &Ticks<T>) -> String {
        let mut out = String::with_capacity(ticks.len() * 160);
        let presence = ticks.columns();
        if self.layout.is_none() {
            let layout = Layout::new(self.option, Some(presence));
            prefix(&mut out, self.format, &layout);
            self.layout = Some(layout);
        }
        if let Some(layout) = &self.layout {
            for (i, tick) in ticks.iter().enumerate() {
                let root = presence
                    .symbols()
                    .and_then(|roots| roots.get(i))
                    .map(|root| &**root)
                    .or_else(|| presence.symbol());
                row(
                    &mut out,
                    self.format,
                    !self.wrote_rows,
                    layout.values(root, tick),
                );
                self.wrote_rows = true;
            }
        }
        out
    }

    /// Close the file once the download ends: write what remains of it, or
    /// report the failure the way the response can still carry it.
    async fn finish(mut self, outcome: Result<(), thetadatadx::Error>) {
        match outcome {
            Ok(()) => {
                let mut out = String::new();
                if self.started.is_some() {
                    // No chunk arrived: the file is its header alone.
                    prefix(&mut out, self.format, &Layout::<T>::new(self.option, None));
                }
                suffix(&mut out, self.format);
                if !out.is_empty() {
                    let _ = self.chunks.send(Ok(Bytes::from(out))).await;
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

fn prefix<T: FlatFileTick>(out: &mut String, format: FlatFileFormat, layout: &Layout<T>) {
    match format {
        FlatFileFormat::Csv => {
            for (i, header) in layout.headers().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(header);
            }
            out.push('\n');
        }
        FlatFileFormat::Json => out.push_str("{\"response\":["),
        FlatFileFormat::Jsonl => {}
        FlatFileFormat::Html => {
            out.push_str("<table><thead><tr>");
            for header in layout.headers() {
                let _ = write!(out, "<th>{header}</th>");
            }
            out.push_str("</tr></thead><tbody>");
        }
    }
}

fn suffix(out: &mut String, format: FlatFileFormat) {
    match format {
        FlatFileFormat::Csv | FlatFileFormat::Jsonl => {}
        FlatFileFormat::Json => out.push_str("]}"),
        FlatFileFormat::Html => out.push_str("</tbody></table>"),
    }
}

fn row<'a>(
    out: &mut String,
    format: FlatFileFormat,
    first: bool,
    values: impl Iterator<Item = (&'static str, Value<'a>)>,
) {
    match format {
        FlatFileFormat::Csv => {
            for (i, (_, value)) in values.enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_csv(out, value);
            }
            out.push('\n');
        }
        FlatFileFormat::Json | FlatFileFormat::Jsonl => {
            if format == FlatFileFormat::Json && !first {
                out.push(',');
            }
            out.push('{');
            for (i, (header, value)) in values.enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "\"{header}\":");
                push_json(out, value);
            }
            out.push('}');
            if format == FlatFileFormat::Jsonl {
                out.push('\n');
            }
        }
        FlatFileFormat::Html => {
            out.push_str("<tr>");
            for (_, value) in values {
                out.push_str("<td>");
                match value {
                    Value::Text(text) => out.push_str(&crate::format::escape_html(text)),
                    Value::Char(c) => out.push_str(&crate::format::escape_html(&c.to_string())),
                    Value::Int(n) => {
                        let _ = write!(out, "{n}");
                    }
                    Value::Price(p) => push_price(out, p),
                    Value::Null => {}
                }
                out.push_str("</td>");
            }
            out.push_str("</tr>");
        }
    }
}

fn push_csv(out: &mut String, value: Value<'_>) {
    match value {
        Value::Text(text) => {
            out.push('"');
            for c in text.chars() {
                if c == '"' {
                    out.push('"');
                }
                out.push(c);
            }
            out.push('"');
        }
        Value::Char(c) => {
            out.push('"');
            out.push(c);
            out.push('"');
        }
        Value::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Price(p) => {
            out.push('"');
            push_price(out, p);
            out.push('"');
        }
        Value::Null => {}
    }
}

fn push_json(out: &mut String, value: Value<'_>) {
    match value {
        Value::Text(text) => push_json_string(out, text),
        Value::Char(c) => push_json_string(out, c.encode_utf8(&mut [0; 4])),
        Value::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Price(p) => {
            out.push('"');
            push_price(out, p);
            out.push('"');
        }
        Value::Null => out.push_str("null"),
    }
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

/// A price the way the terminal writes it: the shortest decimal that reads
/// back as the same number, with `.0` on a whole one.
fn push_price(out: &mut String, price: f64) {
    let start = out.len();
    let _ = write!(out, "{price}");
    if !out[start..].contains('.') {
        out.push_str(".0");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thetadatadx::decode::{self, DecodeError, ResponseSymbol};
    use thetadatadx::wire::{data_value::DataType, DataTable, DataValue, DataValueList, Price};
    use thetadatadx::WireColumns;

    /// Decode a chunk shaped like the terminal's file: its header line, then
    /// rows whose quoted decimals are price cells, other quoted text is text,
    /// and anything bare an integer. The presence set is built the way the
    /// endpoint's stream builds it.
    fn chunk<T: WireColumns>(
        terminal: &str,
        parse: fn(&DataTable) -> Result<Vec<T>, DecodeError>,
    ) -> Ticks<T> {
        let cell = |cell: &str| DataValue {
            data_type: Some(
                match cell.strip_prefix('"').and_then(|c| c.strip_suffix('"')) {
                    None => DataType::Number(cell.parse().expect("integer cell")),
                    Some(text) => match text.split_once('.') {
                        Some((whole, fraction)) => DataType::Price(Price {
                            value: format!("{whole}{fraction}").parse().expect("price cell"),
                            r#type: 10 - i32::try_from(fraction.len()).expect("short fraction"),
                        }),
                        None => DataType::Text(text.to_owned()),
                    },
                },
            ),
        };
        let mut lines = terminal.lines();
        let headers = lines.next().expect("header line").split(',');
        let table = DataTable {
            headers: headers.map(str::to_owned).collect(),
            data_table: lines
                .map(|line| DataValueList {
                    values: line.split(',').map(cell).collect(),
                })
                .collect(),
        };
        let names: Vec<&str> = table.headers.iter().map(String::as_str).collect();
        let columns = T::present_columns(&names);
        let columns = match decode::response_symbol(&table) {
            ResponseSymbol::Constant(root) => columns.with_symbol(root),
            ResponseSymbol::PerRow(roots) => columns.with_symbols(roots),
            ResponseSymbol::Absent => columns,
        };
        Ticks::new(parse(&table).expect("the chunk decodes"), columns)
    }

    fn written<T: FlatFileTick>(format: FlatFileFormat, option: bool, ticks: &Ticks<T>) -> String {
        let (chunks, _body) = mpsc::channel(1);
        let (started, _start) = oneshot::channel();
        let mut out = FileWriter::<T>::new(format, option, chunks, started).render(ticks);
        suffix(&mut out, format);
        out
    }

    /// Each flat file is written the way the terminal writes it: the
    /// terminal's own files for 2026-10-21, decoded from the cells the wire
    /// carries and written out again, come back byte for byte. The quote files
    /// carry a second root, since a chunk of a whole-universe file spans many.
    #[test]
    fn flat_files_are_written_as_the_terminal_writes_them() {
        use FlatFileFormat::{Csv, Json};

        let option_eod = "root,expiration,strike,right,ms_of_day,ms_of_day2,open,high,low,close,volume,count,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"SPY\",20261030,\"808.0\",\"P\",62261420,0,\"0.0\",\"0.0\",\"0.0\",\"0.0\",0,0,1,1,\"33.41\",50,1,1,\"35.92\",50,20261021\n";
        let stock_eod = "root,ms_of_day,ms_of_day2,open,high,low,close,volume,count,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"HCMA\",62128177,57600350,\"10.3\",\"10.3\",\"10.3\",\"10.3\",112241,101,100,7,\"10.26\",0,100,7,\"10.35\",0,20261021\n";
        let index_eod = "root,ms_of_day,ms_of_day2,open,high,low,close,volume,count,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"BCH20RP\",62104186,62103000,\"285.16\",\"290.31\",\"284.32\",\"288.58\",0,0,0,0,\"0.0\",0,0,0,\"0.0\",0,20261021\n";
        for (option, terminal) in [(true, option_eod), (false, stock_eod), (false, index_eod)] {
            let ticks = chunk(terminal, decode::parse_eod_ticks);
            assert_eq!(written(Csv, option, &ticks), terminal);
        }

        let option_trade_quote = "root,expiration,strike,right,ms_of_day,sequence,ext_condition1,ext_condition2,ext_condition3,ext_condition4,condition,size,exchange,price,condition_flags,price_flags,volume_type,records_back,ms_of_day2,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"SPY\",20261030,\"808.0\",\"C\",38866492,996731438,255,255,255,255,134,1,6,\"0.05\",0,7,0,0,38866492,631,6,\"0.05\",50,1217,42,\"0.06\",50,20261021\n";
        let stock_trade_quote = "root,ms_of_day,sequence,ext_condition1,ext_condition2,ext_condition3,ext_condition4,condition,size,exchange,price,condition_flags,price_flags,volume_type,records_back,ms_of_day2,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"HCMA\",34450481,4618650,32,95,255,115,115,3,63,\"10.28\",7,0,0,0,34450481,3000,65,\"10.28\",0,1800,1,\"10.33\",0,20261021\n";
        for (option, terminal) in [(true, option_trade_quote), (false, stock_trade_quote)] {
            let ticks = chunk(terminal, decode::parse_trade_quote_ticks);
            assert_eq!(written(Csv, option, &ticks), terminal);
        }

        let option_open_interest = "root,expiration,strike,right,ms_of_day,open_interest,date\n\
            \"SPY\",20261030,\"808.0\",\"P\",23400563,0,20261021\n";
        let ticks = chunk(option_open_interest, decode::parse_open_interest_ticks);
        assert_eq!(written(Csv, true, &ticks), option_open_interest);

        let option_quote = "root,expiration,strike,right,interval_ms_of_day,quote_ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"ABG\",20270115,\"115.0\",\"C\",34260000,34259636,1,11,\"61.0\",50,1,11,\"65.0\",50,20261021\n\
            \"ABM\",20270115,\"40.0\",\"P\",34320000,34319793,2,4,\"1.95\",50,2,4,\"2.2\",50,20261021\n";
        let stock_quote = "root,interval_ms_of_day,quote_ms_of_day,bid_size,bid_exchange,bid,bid_condition,ask_size,ask_exchange,ask,ask_condition,date\n\
            \"HCMA\",14460000,14401256,100,7,\"10.26\",0,100,7,\"10.35\",0,20261021\n\
            \"HCMB\",25140000,25080173,100,7,\"10.26\",0,100,7,\"10.35\",0,20261021\n";
        for (option, terminal) in [(true, option_quote), (false, stock_quote)] {
            let ticks = chunk(terminal, decode::parse_interval_quote_ticks);
            assert_eq!(written(Csv, option, &ticks), terminal);
        }

        // The JSON form carries the same values: prices as strings, integers
        // as numbers, wrapped in the `response` envelope.
        let ticks = chunk(stock_quote, decode::parse_interval_quote_ticks);
        assert_eq!(
            written(Json, false, &ticks),
            "{\"response\":[\
             {\"root\":\"HCMA\",\"interval_ms_of_day\":14460000,\"quote_ms_of_day\":14401256,\"bid_size\":100,\"bid_exchange\":7,\"bid\":\"10.26\",\"bid_condition\":0,\"ask_size\":100,\"ask_exchange\":7,\"ask\":\"10.35\",\"ask_condition\":0,\"date\":20261021},\
             {\"root\":\"HCMB\",\"interval_ms_of_day\":25140000,\"quote_ms_of_day\":25080173,\"bid_size\":100,\"bid_exchange\":7,\"bid\":\"10.26\",\"bid_condition\":0,\"ask_size\":100,\"ask_exchange\":7,\"ask\":\"10.35\",\"ask_condition\":0,\"date\":20261021}\
             ]}"
        );
    }

    /// A download that fails before any data answers with the failure's own
    /// status; one that fails after data has gone out ends the body in an
    /// error, so the client sees a broken transfer rather than a short file.
    #[tokio::test]
    async fn a_failed_download_is_never_answered_as_a_complete_file() {
        let failure = || thetadatadx::Error::config_internal("upstream reset");
        let stock_eod = "root,ms_of_day,date\n\"HCMA\",62128177,20261021\n";

        let (chunks, mut body) = mpsc::channel(4);
        let (started, start) = oneshot::channel();
        let file = FileWriter::<EodTick>::new(FlatFileFormat::Csv, false, chunks, started);
        file.finish(Err(failure())).await;
        assert!(matches!(start.await, Ok(Err(EndpointError::Server(_)))));
        assert!(body.recv().await.is_none(), "nothing reaches the body");

        let (chunks, mut body) = mpsc::channel(4);
        let (started, start) = oneshot::channel();
        let mut file = FileWriter::<EodTick>::new(FlatFileFormat::Csv, false, chunks, started);
        let sent = file.chunk(&chunk(stock_eod, decode::parse_eod_ticks)).await;
        assert!(sent.is_continue());
        file.finish(Err(failure())).await;
        assert!(matches!(start.await, Ok(Ok(()))));
        assert!(matches!(body.recv().await, Some(Ok(_))));
        assert!(
            matches!(body.recv().await, Some(Err(_))),
            "the body ends in an error"
        );
    }
}
