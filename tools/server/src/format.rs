//! Response rendering for the v3 REST contract.
//!
//! A registry response is written straight from the vendor's table, chunk by
//! chunk, as the rows arrive ([`TableWriter`]). The table carries the
//! terminal's column names; what it does not carry is kept here: each
//! endpoint's columns in the terminal's order ([`Layout`]), and for each
//! column how an empty cell is written and whether it holds a date
//! ([`Kind`]).
//!
//! The JSON body is `{"response":[...]}`. Option tick rows are grouped under
//! their contract, one block for each run of rows on one contract:
//!
//! ```json
//! {
//!     "response": [
//!         { "contract": { "symbol": "AAPL", "expiration": "2026-06-18",
//!                         "strike": 550.000, "right": "CALL" },
//!           "data": [ { "timestamp": "2024-01-02T17:17:53.606", ... } ] }
//!     ]
//! }
//! ```
//!
//! Every other body is flat, with the contract inline. Times are Eastern-time
//! ISO strings, and each price is written at the precision of its cell.

use std::fmt::Write as _;
use std::ops::Range;

use thetadatadx::decode::DecodeError;
use thetadatadx::flatfiles::{HTML_END, HTML_START, escape_html};
use thetadatadx::wire::{DataValueList, data_value::DataType};
use thetadatadx::{EndpointMeta, Price, ReturnType};

use crate::handler::ResponseFormat;

/// Error envelope matching the JVM terminal's error format.
///
/// Canonical shape across every route family (registry endpoints, flat
/// files, rate-limit rejections):
///
/// ```json
/// {
///     "header": { "error_type": "<type>", "error_msg": "<detail>" },
///     "response": []
/// }
/// ```
///
/// Clients parse `header.error_type` to drive retry / backoff logic, so
/// the keys must be identical regardless of which layer produced the
/// failure. The serialise-failure fallbacks in `handler::error_response`
/// and `flatfile_routes::error_response` hand-write the same shape.
pub fn error_envelope(error_type: &str, message: &str) -> sonic_rs::Value {
    sonic_rs::json!({
        "header": {
            "error_type": error_type,
            "error_msg": message
        },
        "response": []
    })
}

/// How a column is written when its cell is empty or missing, and whether it
/// holds a date.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A count the terminal never leaves empty: `0`.
    Count,
    /// A price: `0.0`.
    Price,
    /// A date, written `YYYY-MM-DD` whether it arrives as text, a number or
    /// an instant; empty is null.
    Date,
    /// Anything else, written as it arrives; empty is null.
    Value,
}

#[derive(Clone, Copy)]
struct Column {
    name: &'static str,
    kind: Kind,
}

const fn count(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Count,
    }
}

const fn price(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Price,
    }
}

const fn date(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Date,
    }
}

const fn value(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Value,
    }
}

const CONTRACT: [Column; 4] = [
    value("symbol"),
    date("expiration"),
    price("strike"),
    value("right"),
];
const DATE: [Column; 1] = [date("date")];
const TIMESTAMP: [Column; 1] = [value("timestamp")];
const BAR: [Column; 6] = [
    price("open"),
    price("high"),
    price("low"),
    price("close"),
    count("volume"),
    count("count"),
];
const QUOTE: [Column; 8] = [
    count("bid_size"),
    value("bid_exchange"),
    price("bid"),
    value("bid_condition"),
    count("ask_size"),
    value("ask_exchange"),
    price("ask"),
    value("ask_condition"),
];
const TRADE: [Column; 9] = [
    count("sequence"),
    value("ext_condition1"),
    value("ext_condition2"),
    value("ext_condition3"),
    value("ext_condition4"),
    value("condition"),
    count("size"),
    value("exchange"),
    price("price"),
];
const BID_ASK: [Column; 2] = [price("bid"), price("ask")];
const ALL_GREEKS: [Column; 20] = [
    price("delta"),
    price("theta"),
    price("vega"),
    price("rho"),
    price("epsilon"),
    price("lambda"),
    price("gamma"),
    price("vanna"),
    price("charm"),
    price("vomma"),
    price("veta"),
    price("vera"),
    price("speed"),
    price("zomma"),
    price("color"),
    price("ultima"),
    price("d1"),
    price("d2"),
    price("dual_delta"),
    price("dual_gamma"),
];
const SECOND_ORDER: [Column; 5] = [
    price("gamma"),
    price("vanna"),
    price("charm"),
    price("vomma"),
    price("veta"),
];
const THIRD_ORDER: [Column; 4] = [
    price("speed"),
    price("zomma"),
    price("color"),
    price("ultima"),
];
const IV_UNDERLYING: [Column; 4] = [
    price("implied_vol"),
    price("iv_error"),
    value("underlying_timestamp"),
    price("underlying_price"),
];

/// The columns one endpoint writes, in the terminal's order.
struct Layout {
    /// The columns that say what a row is about: the contract of an option
    /// row, the symbol of a snapshot row, the day of a calendar row. Each is
    /// written only when the vendor sends it.
    keys: &'static [Column],
    /// The keys follow the row's leading timestamp instead of leading it.
    keys_after_timestamp: bool,
    /// Every other column, written on every row.
    body: Vec<Column>,
    /// The columns a table with rows must carry. Without one, a row has no
    /// time or no value, and the response is refused rather than written
    /// with the column blank.
    required: &'static [&'static str],
    /// The symbol-scoped lists pair each value with the requested symbol,
    /// which their answer does not repeat.
    request_symbol: bool,
    /// The JSON body groups the rows under their contract.
    grouped: bool,
}

impl Layout {
    fn of(ep: &EndpointMeta) -> Self {
        let snapshot = ep.rest_path.contains("/snapshot/");
        let option_tick =
            ep.rest_path.contains("/option/") && !ep.rest_path.contains("/option/list/");
        let (keys, keys_after_timestamp): (&'static [Column], bool) = if option_tick {
            // Option rows lead with the contract, except on the four
            // snapshots the terminal writes timestamp first.
            let timestamp_first = matches!(
                ep.name,
                "option_snapshot_ohlc"
                    | "option_snapshot_market_value"
                    | "option_snapshot_open_interest"
                    | "option_snapshot_quote"
            );
            (&CONTRACT, timestamp_first)
        } else if snapshot {
            (&CONTRACT[..1], true)
        } else if ep.returns == ReturnType::CalendarDays {
            (&DATE, false)
        } else {
            (&[], false)
        };
        let list = |suffix: &str| ep.name.ends_with(suffix);
        let (body, required): (&[&[Column]], &'static [&'static str]) = match ep.returns {
            ReturnType::StringList if list("_list_dates") => (&[&DATE], &["date"]),
            ReturnType::StringList if list("_list_expirations") => {
                (&[&CONTRACT[1..2]], &["expiration"])
            }
            ReturnType::StringList if list("_list_strikes") => (&[&CONTRACT[2..3]], &["strike"]),
            ReturnType::StringList => (&[&CONTRACT[..1]], &["symbol"]),
            ReturnType::OptionContracts => (&[&CONTRACT], &["symbol"]),
            ReturnType::CalendarDays => (
                &[&[value("type"), value("open"), value("close")]],
                &["type"],
            ),
            ReturnType::InterestRateTicks => (&[&[date("created"), price("rate")]], &["created"]),
            ReturnType::EodTicks => (
                &[&[value("created"), value("last_trade")], &BAR, &QUOTE],
                &["created"],
            ),
            ReturnType::OhlcTicks if snapshot => (&[&TIMESTAMP, &BAR], &["timestamp", "open"]),
            ReturnType::OhlcTicks => (
                &[&TIMESTAMP, &BAR, &[price("vwap")]],
                &["timestamp", "open"],
            ),
            // A stock or index snapshot trade is the last trade's summary,
            // not the whole execution record.
            ReturnType::TradeTicks if snapshot && !option_tick => (
                &[
                    &TIMESTAMP,
                    &[
                        count("sequence"),
                        count("size"),
                        value("condition"),
                        price("price"),
                    ],
                ],
                &["timestamp", "price"],
            ),
            ReturnType::TradeTicks | ReturnType::IndexPriceAtTimeTicks => {
                (&[&TIMESTAMP, &TRADE], &["timestamp", "price"])
            }
            ReturnType::TradeQuoteTicks => (
                &[
                    &[value("trade_timestamp"), value("quote_timestamp")],
                    &TRADE,
                    &QUOTE,
                ],
                &["trade_timestamp", "price"],
            ),
            ReturnType::QuoteTicks => (&[&TIMESTAMP, &QUOTE], &["timestamp", "bid", "ask"]),
            ReturnType::OpenInterestTicks => {
                (&[&TIMESTAMP, &[count("open_interest")]], &["timestamp"])
            }
            ReturnType::MarketValueTicks if ep.rest_path.contains("/index/") => {
                (&[&TIMESTAMP, &[price("market_price")]], &["timestamp"])
            }
            ReturnType::MarketValueTicks => (
                &[
                    &TIMESTAMP,
                    &[
                        price("market_bid"),
                        price("market_ask"),
                        price("market_price"),
                    ],
                ],
                &["timestamp"],
            ),
            ReturnType::GreeksAllTicks => (
                &[&TIMESTAMP, &BID_ASK, &ALL_GREEKS, &IV_UNDERLYING],
                &["timestamp"],
            ),
            ReturnType::GreeksEodTicks => (
                &[&TIMESTAMP, &BAR, &QUOTE, &ALL_GREEKS, &IV_UNDERLYING],
                &["timestamp"],
            ),
            ReturnType::GreeksFirstOrderTicks => (
                &[&TIMESTAMP, &BID_ASK, &ALL_GREEKS[..6], &IV_UNDERLYING],
                &["timestamp"],
            ),
            ReturnType::GreeksSecondOrderTicks => (
                &[&TIMESTAMP, &BID_ASK, &SECOND_ORDER, &IV_UNDERLYING],
                &["timestamp"],
            ),
            ReturnType::GreeksThirdOrderTicks => (
                &[&TIMESTAMP, &BID_ASK, &THIRD_ORDER, &IV_UNDERLYING],
                &["timestamp"],
            ),
            ReturnType::TradeGreeksAllTicks => (
                &[&TIMESTAMP, &TRADE, &ALL_GREEKS, &IV_UNDERLYING],
                &["timestamp", "price"],
            ),
            ReturnType::TradeGreeksFirstOrderTicks => (
                &[&TIMESTAMP, &TRADE, &ALL_GREEKS[..6], &IV_UNDERLYING],
                &["timestamp", "price"],
            ),
            ReturnType::TradeGreeksSecondOrderTicks => (
                &[&TIMESTAMP, &TRADE, &SECOND_ORDER, &IV_UNDERLYING],
                &["timestamp", "price"],
            ),
            ReturnType::TradeGreeksThirdOrderTicks => (
                &[&TIMESTAMP, &TRADE, &THIRD_ORDER, &IV_UNDERLYING],
                &["timestamp", "price"],
            ),
            ReturnType::TradeGreeksImpliedVolatilityTicks => (
                &[&TIMESTAMP, &TRADE, &IV_UNDERLYING],
                &["timestamp", "price"],
            ),
            ReturnType::IvTicks if snapshot => {
                (&[&TIMESTAMP, &BID_ASK, &IV_UNDERLYING], &["timestamp"])
            }
            ReturnType::IvTicks => (
                &[
                    &TIMESTAMP,
                    &[
                        price("bid"),
                        price("bid_implied_vol"),
                        price("midpoint"),
                        price("implied_vol"),
                        price("ask"),
                        price("ask_implied_vol"),
                    ],
                    &IV_UNDERLYING[1..],
                ],
                &["timestamp"],
            ),
            ReturnType::PriceTicks => (&[&TIMESTAMP, &[price("price")]], &["timestamp", "price"]),
            // Only the quote flat files return interval quotes, and the
            // router serves every flat file through `flatfile_routes`.
            ReturnType::IntervalQuoteTicks => {
                unreachable!("{} is a flat file, served by flatfile_routes", ep.name)
            }
        };
        Self {
            keys,
            keys_after_timestamp,
            body: body.concat(),
            required,
            request_symbol: list("_list_expirations") || list("_list_strikes"),
            grouped: option_tick,
        }
    }
}

/// Where a written column's cells come from.
#[derive(Clone, Copy)]
enum Source {
    /// This column of the vendor's rows.
    Wire(usize),
    /// A column the vendor did not send.
    Absent,
    /// The requested symbol.
    Request,
}

/// One cell, resolved to what is written for it.
enum Value<'a> {
    /// No value: `null` in JSON and HTML, an empty CSV field.
    Null,
    /// The vendor's text, escaped as the format needs.
    Text(&'a str),
    /// A date or a time: quoted in JSON, and never in need of escaping.
    Time(&'a str),
    /// A number, never quoted.
    Number(&'a str),
}

/// Writes one response, in one format, from the vendor's table as its chunks
/// arrive.
pub(crate) struct TableWriter {
    format: ResponseFormat,
    layout: Layout,
    /// The requested symbol, written on each row of a symbol-scoped list.
    request_symbol: String,
    /// The columns written and where each comes from.
    columns: Vec<(Column, Source)>,
    /// Where the contract sits among `columns`, for the JSON grouping.
    contract: Range<usize>,
    rows: usize,
    /// The contract of the JSON block being written, and of the row being
    /// placed.
    block: Vec<u8>,
    next_block: Vec<u8>,
    /// `json_legacy` only: each column so far, from its key to its last cell.
    legacy: Vec<Vec<u8>>,
    scratch: String,
}

impl TableWriter {
    /// A writer for one response of `ep`. `symbol` is the request's `symbol`
    /// parameter.
    pub(crate) fn new(ep: &EndpointMeta, format: ResponseFormat, symbol: Option<&str>) -> Self {
        let layout = Layout::of(ep);
        // The lists refuse more than one distinct symbol, so the first named
        // is the one.
        let request_symbol = symbol
            .filter(|_| layout.request_symbol)
            .and_then(|symbol| symbol.split(',').map(str::trim).find(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_owned();
        Self {
            format,
            layout,
            request_symbol,
            columns: Vec::new(),
            contract: 0..0,
            rows: 0,
            block: Vec::new(),
            next_block: Vec::new(),
            legacy: Vec::new(),
            scratch: String::new(),
        }
    }

    /// Append one chunk's rows to `out`, after whatever opens the body when
    /// they are its first.
    ///
    /// # Errors
    ///
    /// A chunk with rows that lacks a column the endpoint requires, a row
    /// shorter than its headers, a cell with no value, a price type outside
    /// `0..=19`, or an instant outside the calendar the conversion covers.
    pub(crate) fn write(
        &mut self,
        headers: &[String],
        rows: &[DataValueList],
        out: &mut Vec<u8>,
    ) -> Result<(), DecodeError> {
        if self.rows == 0 {
            self.place(headers);
            if let Some(header) = self
                .layout
                .required
                .iter()
                .copied()
                .find(|name| !headers.iter().any(|header| header == name))
                .filter(|_| !rows.is_empty())
            {
                return Err(DecodeError::MissingRequiredHeader {
                    header,
                    rows: rows.len(),
                    available: headers.join(","),
                });
            }
        }
        for row in rows {
            let first = self.rows == 0;
            self.rows += 1;
            match self.format {
                ResponseFormat::Csv => {
                    if first {
                        for (i, (column, _)) in self.columns.iter().enumerate() {
                            if i > 0 {
                                out.push(b',');
                            }
                            out.extend_from_slice(column.name.as_bytes());
                        }
                        out.extend_from_slice(b"\r\n");
                    }
                    for (i, &(column, source)) in self.columns.iter().enumerate() {
                        if i > 0 {
                            out.push(b',');
                        }
                        let value =
                            cell(row, column, source, &self.request_symbol, &mut self.scratch)?;
                        push_value(out, self.format, value);
                    }
                    out.extend_from_slice(b"\r\n");
                }
                ResponseFormat::Html => {
                    if first {
                        out.extend_from_slice(HTML_START.as_bytes());
                        out.extend_from_slice(b"<thead><tr>");
                        for (column, _) in &self.columns {
                            out.extend_from_slice(b"<th>");
                            out.extend_from_slice(column.name.as_bytes());
                            out.extend_from_slice(b"</th>");
                        }
                        out.extend_from_slice(b"</tr></thead>\n<tbody>");
                    }
                    out.extend_from_slice(b"<tr>");
                    for &(column, source) in &self.columns {
                        out.extend_from_slice(b"<td>");
                        let value =
                            cell(row, column, source, &self.request_symbol, &mut self.scratch)?;
                        push_value(out, self.format, value);
                        out.extend_from_slice(b"</td>");
                    }
                    out.extend_from_slice(b"</tr>\n");
                }
                ResponseFormat::Ndjson => {
                    push_object(
                        out,
                        &self.columns,
                        row,
                        &self.request_symbol,
                        &mut self.scratch,
                    )?;
                    out.push(b'\n');
                }
                ResponseFormat::Json if self.layout.grouped => {
                    self.next_block.clear();
                    let contract = &self.columns[self.contract.clone()];
                    push_object(
                        &mut self.next_block,
                        contract,
                        row,
                        &self.request_symbol,
                        &mut self.scratch,
                    )?;
                    if first || self.next_block != self.block {
                        let open = if first { "{\"response\":[" } else { "]}," };
                        out.extend_from_slice(open.as_bytes());
                        out.extend_from_slice(b"{\"contract\":");
                        out.extend_from_slice(&self.next_block);
                        out.extend_from_slice(b",\"data\":[");
                        std::mem::swap(&mut self.block, &mut self.next_block);
                    } else {
                        out.push(b',');
                    }
                    let data = self.columns[..self.contract.start]
                        .iter()
                        .chain(&self.columns[self.contract.end..]);
                    push_object(out, data, row, &self.request_symbol, &mut self.scratch)?;
                }
                ResponseFormat::Json => {
                    out.extend_from_slice(if first { "{\"response\":[" } else { "," }.as_bytes());
                    push_object(
                        out,
                        &self.columns,
                        row,
                        &self.request_symbol,
                        &mut self.scratch,
                    )?;
                }
                ResponseFormat::JsonLegacy => {
                    if first {
                        self.legacy = self
                            .columns
                            .iter()
                            .map(|(column, _)| format!("\"{}\":[", column.name).into_bytes())
                            .collect();
                    }
                    for (values, &(column, source)) in self.legacy.iter_mut().zip(&self.columns) {
                        if !first {
                            values.push(b',');
                        }
                        let value =
                            cell(row, column, source, &self.request_symbol, &mut self.scratch)?;
                        push_value(values, self.format, value);
                    }
                }
            }
        }
        Ok(())
    }

    /// Append what closes the body once the last chunk is written: the end of
    /// the JSON envelope or of the HTML page, or the whole `json_legacy` body,
    /// whose columns cannot close before the last row.
    pub(crate) fn finish(&self, out: &mut Vec<u8>) {
        match self.format {
            ResponseFormat::Csv | ResponseFormat::Ndjson => {}
            ResponseFormat::Html if self.rows > 0 => out.extend_from_slice(HTML_END.as_bytes()),
            ResponseFormat::Html => {}
            ResponseFormat::Json if self.rows == 0 => out.extend_from_slice(b"{\"response\":[]}"),
            ResponseFormat::Json if self.layout.grouped => out.extend_from_slice(b"]}]}"),
            ResponseFormat::Json => out.extend_from_slice(b"]}"),
            ResponseFormat::JsonLegacy => {
                out.push(b'{');
                for (i, column) in self.legacy.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(column);
                    out.push(b']');
                }
                out.push(b'}');
            }
        }
    }

    /// Place the endpoint's columns among the vendor's headers.
    fn place(&mut self, headers: &[String]) {
        let wire = |column: &Column| headers.iter().position(|header| header == column.name);
        let keys: Vec<(Column, Source)> = self
            .layout
            .keys
            .iter()
            .filter_map(|column| Some((*column, Source::Wire(wire(column)?))))
            .collect();
        self.columns.clear();
        if !self.request_symbol.is_empty() {
            self.columns.push((CONTRACT[0], Source::Request));
        }
        self.columns.extend(
            self.layout
                .body
                .iter()
                .map(|column| (*column, wire(column).map_or(Source::Absent, Source::Wire))),
        );
        let at = usize::from(self.layout.keys_after_timestamp).min(self.columns.len());
        self.contract = at..at + keys.len();
        self.columns.splice(at..at, keys);
    }
}

/// The value written for `column` of `row`.
fn cell<'a>(
    row: &'a DataValueList,
    column: Column,
    source: Source,
    request_symbol: &'a str,
    scratch: &'a mut String,
) -> Result<Value<'a>, DecodeError> {
    let data = match source {
        Source::Request => return Ok(Value::Text(request_symbol)),
        Source::Absent => None,
        Source::Wire(index) => Some(
            row.values
                .get(index)
                .and_then(|cell| cell.data_type.as_ref())
                .ok_or(DecodeError::MissingCell { column: index })?,
        ),
    };
    scratch.clear();
    Ok(match data {
        None | Some(DataType::NullValue(_)) => match column.kind {
            Kind::Count => Value::Number("0"),
            Kind::Price => Value::Number("0.0"),
            Kind::Date | Kind::Value => Value::Null,
        },
        Some(DataType::Text(text))
            if column.kind == Kind::Date
                && text.len() == 8
                && text.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            let _ = write!(scratch, "{}-{}-{}", &text[..4], &text[4..6], &text[6..]);
            Value::Time(scratch)
        }
        Some(DataType::Text(text)) => Value::Text(text),
        Some(DataType::Number(number)) => match i32::try_from(*number) {
            Ok(day) if column.kind == Kind::Date && thetadatadx::time::is_valid_yyyymmdd(day) => {
                push_date(scratch, day);
                Value::Time(scratch)
            }
            _ => {
                let _ = write!(scratch, "{number}");
                Value::Number(scratch)
            }
        },
        Some(DataType::Price(cell)) => {
            let price = Price::with_value_and_type(cell.value, cell.r#type)
                .map_err(|_| DecodeError::InvalidPriceType { raw: cell.r#type })?;
            if cell.value == 0 && cell.r#type > 10 {
                // Its digits would read `00.0`, which is not a number.
                Value::Number("0.0")
            } else {
                let _ = write!(scratch, "{price}");
                Value::Number(scratch)
            }
        }
        Some(DataType::Timestamp(instant)) => {
            let out_of_range = || DecodeError::TimestampOutOfRange {
                raw: instant.epoch_ms,
            };
            let day = thetadatadx::time::try_timestamp_to_date(instant.epoch_ms)
                .ok_or_else(out_of_range)?;
            push_date(scratch, day);
            if column.kind != Kind::Date {
                let ms = thetadatadx::time::try_timestamp_to_ms_of_day(instant.epoch_ms)
                    .ok_or_else(out_of_range)?;
                // The terminal always writes three digits of milliseconds.
                let _ = write!(
                    scratch,
                    "T{:02}:{:02}:{:02}.{:03}",
                    ms / 3_600_000,
                    ms / 60_000 % 60,
                    ms / 1_000 % 60,
                    ms % 1_000
                );
            }
            Value::Time(scratch)
        }
    })
}

/// Write a `YYYYMMDD` day as `YYYY-MM-DD`.
fn push_date(out: &mut String, day: i32) {
    let _ = write!(
        out,
        "{:04}-{:02}-{:02}",
        day / 10_000,
        day / 100 % 100,
        day % 100
    );
}

/// Write `columns` of `row` as one JSON object.
fn push_object<'c>(
    out: &mut Vec<u8>,
    columns: impl IntoIterator<Item = &'c (Column, Source)>,
    row: &DataValueList,
    request_symbol: &str,
    scratch: &mut String,
) -> Result<(), DecodeError> {
    out.push(b'{');
    for (i, &(column, source)) in columns.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.push(b'"');
        out.extend_from_slice(column.name.as_bytes());
        out.extend_from_slice(b"\":");
        let value = cell(row, column, source, request_symbol, scratch)?;
        push_value(out, ResponseFormat::Json, value);
    }
    out.push(b'}');
    Ok(())
}

/// Write one cell as `format` spells it.
fn push_value(out: &mut Vec<u8>, format: ResponseFormat, value: Value<'_>) {
    let json = !matches!(format, ResponseFormat::Csv | ResponseFormat::Html);
    match value {
        Value::Null if format == ResponseFormat::Csv => {}
        Value::Null => out.extend_from_slice(b"null"),
        Value::Text(text) if json => {
            sonic_rs::to_writer(&mut *out, text).expect("a string serialises into a byte buffer");
        }
        Value::Text(text) if format == ResponseFormat::Html => {
            out.extend_from_slice(escape_html(text).as_bytes());
        }
        // RFC 4180: a field holding a comma, a quote or a line break is
        // quoted, with each quote inside it doubled.
        Value::Text(text) if text.contains([',', '"', '\n', '\r']) => {
            out.push(b'"');
            out.extend_from_slice(text.replace('"', "\"\"").as_bytes());
            out.push(b'"');
        }
        Value::Time(text) if json => {
            out.push(b'"');
            out.extend_from_slice(text.as_bytes());
            out.push(b'"');
        }
        Value::Text(text) | Value::Time(text) | Value::Number(text) => {
            out.extend_from_slice(text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};

    /// The error envelope must carry `header.error_type` + `header.error_msg`
    /// with an empty `response` array, the same shape the JVM terminal
    /// emits and the flat-file / handler fallback strings hand-write. The
    /// nested `error.message` form must never come back: clients parse one
    /// shape across every route family.
    #[test]
    fn error_envelope_uses_canonical_error_msg_shape() {
        let envelope = error_envelope("bad_request", "missing required parameter: 'date'");

        let header = envelope
            .get("header")
            .and_then(|h: &sonic_rs::Value| h.as_object())
            .expect("envelope must carry a header object");
        assert_eq!(
            header.get(&"error_type").and_then(sonic_rs::Value::as_str),
            Some("bad_request")
        );
        assert_eq!(
            header.get(&"error_msg").and_then(sonic_rs::Value::as_str),
            Some("missing required parameter: 'date'")
        );

        let response = envelope
            .get("response")
            .and_then(|r: &sonic_rs::Value| r.as_array())
            .expect("envelope must carry a response array");
        assert!(response.is_empty(), "error envelope response must be []");

        assert!(
            envelope.get("error").is_none(),
            "nested error.message form must not be emitted"
        );
    }
}
