//! Format-pluggable row writer for the FLATFILES surface.
//!
//! Each output format implements [`RowSink`], a tiny three-method
//! interface: write the header once, accept rows one-by-one with the
//! contract key + decoded data fields, and finalize on completion. The
//! decoder layer (see `decoded.rs`) drives a single sink regardless of
//! format — CSV, JSONL, JSON array, or HTML table.
//!
//! The contract key passed to each sink call carries the
//! `(root, exp, strike, right)` columns the vendor prepends to every CSV
//! row. For stock entries, `exp / strike / right` are `None` and only
//! `root` is written.
//!
//! Every sink produces the **same logical rows**; only the on-disk
//! encoding differs. This is what makes the byte-match test on `Csv` a
//! sufficient proxy for the row-level correctness of the others.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::Error;
use crate::flatfiles::datatype::DataType;
use crate::flatfiles::index::IndexEntry;
use crate::flatfiles::types::SecType;

/// Shape of a row passed into the sink.
///
/// `data` is exactly the per-blob FIT column schema (with PRICE_TYPE
/// still in place, if present). Each sink is responsible for skipping
/// the PRICE_TYPE column from its visible output.
pub(crate) struct RowView<'a> {
    pub(crate) entry: &'a IndexEntry,
    pub(crate) data: &'a [i32],
}

/// Polymorphic row writer.
pub(crate) trait RowSink {
    /// Emits any once-per-file header. Called before the first row.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` when the underlying writer fails.
    fn write_header(&mut self) -> Result<(), Error>;
    /// Emits one decoded row.
    ///
    /// # Errors
    ///
    /// Returns `Error::decode_codec` when a price column carries an
    /// out-of-range PRICE_TYPE, or `Error::Io` / encode errors from the
    /// underlying writer.
    fn write_row(&mut self, row: RowView<'_>) -> Result<(), Error>;
    /// Flushes and finalizes the sink, consuming it.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` when the final flush fails.
    fn finish(self: Box<Self>) -> Result<(), Error>;
}

// ---------------------------------------------------------------------------
// Shared format helpers
// ---------------------------------------------------------------------------

/// Indices of the data-only columns in a row (i.e. all columns except
/// the optional PRICE_TYPE column). Computed once from the schema and
/// reused per row to skip work in the hot loop.
pub(crate) fn data_indices(fmt: &[DataType], price_type_idx: Option<usize>) -> Vec<usize> {
    fmt.iter()
        .enumerate()
        .filter_map(|(i, _)| {
            if Some(i) == price_type_idx {
                None
            } else {
                Some(i)
            }
        })
        .collect()
}

/// Per-row PRICE_TYPE exponent for `crate::tdbe::types::price::Price` decoding.
///
/// When PRICE_TYPE is in the schema, the value at that column is the
/// vendor `price_type` field (real price = `value * 10^(price_type - 10)`).
/// When PRICE_TYPE is absent, the column is not a price (the vendor's
/// `toCSV2` does not call `PriceCalc.fmtPrice` in that branch), so the
/// integer value is emitted unchanged.
pub(crate) fn price_type_for_row(row: &[i32], price_type_idx: Option<usize>) -> Option<i32> {
    price_type_idx.map(|idx| row.get(idx).copied().unwrap_or(0))
}

/// Convert a wire `(value, price_type)` pair to its real f64 price
/// using the canonical [`crate::tdbe::types::price::Price`] semantics. Returns
/// `0.0` for `price_type == 0` (vendor sentinel for "no price").
///
/// # Errors
///
/// Returns `Error::decode_codec(..)` when `price_type` is outside
/// `0..=crate::tdbe::types::price::MAX_PRICE_TYPE`. The raw wire value is
/// captured in the message.
pub(crate) fn decode_price(integer: i32, price_type: i32) -> Result<f64, Error> {
    crate::tdbe::types::price::Price::with_value_and_type(integer, price_type)
        .map(|p| p.to_f64())
        .map_err(|_| {
            Error::decode_codec(format!(
                "flatfile price_type {price_type} outside valid range [0, {}]",
                crate::tdbe::types::price::MAX_PRICE_TYPE
            ))
        })
}

/// Render the decoded price into `buf` using `f64` Display, which
/// preserves full IEEE-754 precision. Required for sub-cent options,
/// where fixed-point rendering rounds to zero.
///
/// # Errors
///
/// Propagates `Error::decode_codec(..)` from [`decode_price`].
pub(crate) fn fmt_price_into(buf: &mut String, integer: i32, price_type: i32) -> Result<(), Error> {
    use std::fmt::Write;
    let v = decode_price(integer, price_type)?;
    let _ = write!(buf, "{v}");
    Ok(())
}

/// Encode one row as a JSON object with the same keys and values the
/// JSONL / JSON-array sinks emit: the contract prefix
/// (`symbol[,expiration,strike,right]`) followed by the decoded data
/// columns. Prices decode via [`decode_price`]; every other column stays
/// an integer.
///
/// # Errors
///
/// Propagates [`decode_price`] failures on an out-of-range PRICE_TYPE.
fn encode_row_object(
    entry: &IndexEntry,
    sec: SecType,
    fmt: &[DataType],
    data_idx: &[usize],
    price_type_idx: Option<usize>,
    data: &[i32],
) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
    let mut obj = serde_json::Map::with_capacity(data_idx.len() + 4);
    match sec {
        SecType::Option => {
            obj.insert(
                "symbol".into(),
                serde_json::Value::String(entry.symbol.clone()),
            );
            obj.insert(
                "expiration".into(),
                serde_json::Value::Number(entry.expiration.unwrap_or(0).into()),
            );
            // Strikes are dollars on every client-facing surface; emit the
            // same dollar value as the CSV and Arrow paths via the shared
            // conversion. `from_f64` is the only fallible step, so fall back
            // to JSON null defensively (an i32/1000 quotient is always finite).
            let strike_dollars = entry.strike_dollars().unwrap_or(0.0);
            obj.insert(
                "strike".into(),
                serde_json::Number::from_f64(strike_dollars)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert(
                "right".into(),
                serde_json::Value::String(entry.right.unwrap_or('?').to_string()),
            );
        }
        SecType::Stock | SecType::Index => {
            obj.insert(
                "symbol".into(),
                serde_json::Value::String(entry.symbol.clone()),
            );
        }
    }
    let pt = price_type_for_row(data, price_type_idx);
    for &i in data_idx {
        let val = data.get(i).copied().unwrap_or(0);
        let v = if fmt[i].is_price() {
            if let Some(t) = pt {
                let f = decode_price(val, t)?;
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Number(val.into())
            }
        } else {
            serde_json::Value::Number(val.into())
        };
        obj.insert(fmt[i].name().into_owned(), v);
    }
    Ok(obj)
}

/// Append the HTML-escaped form of `s` to `buf` (`&`→`&amp;`, `<`→`&lt;`,
/// `>`→`&gt;`, `"`→`&quot;`).
fn append_html_escaped(buf: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => buf.push_str("&amp;"),
            '<' => buf.push_str("&lt;"),
            '>' => buf.push_str("&gt;"),
            '"' => buf.push_str("&quot;"),
            other => buf.push(other),
        }
    }
}

/// Build the contract-prefix segment of a CSV row.
fn append_csv_prefix(buf: &mut String, entry: &IndexEntry, sec: SecType) {
    use std::fmt::Write;
    match sec {
        SecType::Option => {
            buf.push_str(&entry.symbol);
            buf.push(',');
            let _ = write!(buf, "{}", entry.expiration.unwrap_or(0));
            buf.push(',');
            // Strikes are dollars on every client-facing surface — emit
            // the same dollar value the Arrow and typed-row paths do, via
            // the shared conversion. `f64` Display preserves sub-dollar
            // strikes without trailing-zero noise.
            let _ = write!(buf, "{}", entry.strike_dollars().unwrap_or(0.0));
            buf.push(',');
            buf.push(entry.right.unwrap_or('?'));
            buf.push(',');
        }
        SecType::Stock | SecType::Index => {
            // Stock and index entries carry only the symbol — no expiration,
            // strike, or right dimension.
            buf.push_str(&entry.symbol);
            buf.push(',');
        }
    }
}

// ---------------------------------------------------------------------------
// CSV sink — vendor byte format
// ---------------------------------------------------------------------------

/// [`RowSink`] that writes the vendor CSV byte format.
pub(crate) struct CsvSink {
    out: BufWriter<File>,
    sec: SecType,
    fmt: Vec<DataType>,
    data_idx: Vec<usize>,
    price_type_idx: Option<usize>,
    /// Reused row buffer to avoid per-row allocation.
    line: String,
}

impl CsvSink {
    /// Creates a CSV sink writing to `path`.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` when `path` cannot be created.
    pub(crate) fn new(
        path: &Path,
        sec: SecType,
        fmt: Vec<DataType>,
        price_type_idx: Option<usize>,
    ) -> Result<Self, Error> {
        let data_idx = data_indices(&fmt, price_type_idx);
        let f = File::create(path)?;
        Ok(Self {
            out: BufWriter::with_capacity(1 << 20, f),
            sec,
            fmt,
            data_idx,
            price_type_idx,
            line: String::with_capacity(256),
        })
    }
}

impl RowSink for CsvSink {
    fn write_header(&mut self) -> Result<(), Error> {
        self.line.clear();
        match self.sec {
            SecType::Option => {
                self.line.push_str("symbol,expiration,strike,right,");
            }
            SecType::Stock | SecType::Index => self.line.push_str("symbol,"),
        }
        for (n, &i) in self.data_idx.iter().enumerate() {
            if n > 0 {
                self.line.push(',');
            }
            self.line.push_str(&self.fmt[i].name());
        }
        self.line.push('\n');
        self.out.write_all(self.line.as_bytes())?;
        Ok(())
    }

    fn write_row(&mut self, row: RowView<'_>) -> Result<(), Error> {
        self.line.clear();
        append_csv_prefix(&mut self.line, row.entry, self.sec);
        let pt = price_type_for_row(row.data, self.price_type_idx);
        for (n, &i) in self.data_idx.iter().enumerate() {
            if n > 0 {
                self.line.push(',');
            }
            let val = row.data.get(i).copied().unwrap_or(0);
            if self.fmt[i].is_price() {
                if let Some(t) = pt {
                    fmt_price_into(&mut self.line, val, t)?;
                } else {
                    use std::fmt::Write;
                    let _ = write!(self.line, "{val}");
                }
            } else {
                use std::fmt::Write;
                let _ = write!(self.line, "{val}");
            }
        }
        self.line.push('\n');
        self.out.write_all(self.line.as_bytes())?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), Error> {
        self.out.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// JSONL sink — one JSON object per line
// ---------------------------------------------------------------------------

/// [`RowSink`] that writes one JSON object per line (JSONL).
pub(crate) struct JsonlSink {
    out: BufWriter<File>,
    sec: SecType,
    fmt: Vec<DataType>,
    data_idx: Vec<usize>,
    price_type_idx: Option<usize>,
}

impl JsonlSink {
    /// Creates a JSONL sink writing to `path`.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` when `path` cannot be created.
    pub(crate) fn new(
        path: &Path,
        sec: SecType,
        fmt: Vec<DataType>,
        price_type_idx: Option<usize>,
    ) -> Result<Self, Error> {
        let data_idx = data_indices(&fmt, price_type_idx);
        let f = File::create(path)?;
        Ok(Self {
            out: BufWriter::with_capacity(1 << 20, f),
            sec,
            fmt,
            data_idx,
            price_type_idx,
        })
    }
}

impl RowSink for JsonlSink {
    fn write_header(&mut self) -> Result<(), Error> {
        // JSONL has no header row; nothing to emit. Object keys are
        // written per-row alongside their values.
        Ok(())
    }

    fn write_row(&mut self, row: RowView<'_>) -> Result<(), Error> {
        let obj = encode_row_object(
            row.entry,
            self.sec,
            &self.fmt,
            &self.data_idx,
            self.price_type_idx,
            row.data,
        )?;
        serde_json::to_writer(&mut self.out, &serde_json::Value::Object(obj))
            .map_err(|e| Error::config_internal(format!("flatfiles: jsonl encode failed: {e}")))?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), Error> {
        self.out.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// JSON-array sink — a single streamed JSON array
// ---------------------------------------------------------------------------

/// [`RowSink`] that streams a single JSON array of the same per-row
/// objects [`JsonlSink`] emits: `[` on header, one object per row joined
/// by `,`, `]` on finish. Never buffers the whole document.
pub(crate) struct JsonArraySink {
    out: BufWriter<File>,
    sec: SecType,
    fmt: Vec<DataType>,
    data_idx: Vec<usize>,
    price_type_idx: Option<usize>,
    /// `false` once the first element has been written, so subsequent
    /// rows are prefixed with the element separator `,`.
    first: bool,
}

impl JsonArraySink {
    /// Creates a JSON-array sink writing to `path`.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` when `path` cannot be created.
    pub(crate) fn new(
        path: &Path,
        sec: SecType,
        fmt: Vec<DataType>,
        price_type_idx: Option<usize>,
    ) -> Result<Self, Error> {
        let data_idx = data_indices(&fmt, price_type_idx);
        let f = File::create(path)?;
        Ok(Self {
            out: BufWriter::with_capacity(1 << 20, f),
            sec,
            fmt,
            data_idx,
            price_type_idx,
            first: true,
        })
    }
}

impl RowSink for JsonArraySink {
    fn write_header(&mut self) -> Result<(), Error> {
        self.out.write_all(b"[")?;
        Ok(())
    }

    fn write_row(&mut self, row: RowView<'_>) -> Result<(), Error> {
        if self.first {
            self.first = false;
        } else {
            self.out.write_all(b",")?;
        }
        let obj = encode_row_object(
            row.entry,
            self.sec,
            &self.fmt,
            &self.data_idx,
            self.price_type_idx,
            row.data,
        )?;
        serde_json::to_writer(&mut self.out, &serde_json::Value::Object(obj))
            .map_err(|e| Error::config_internal(format!("flatfiles: json encode failed: {e}")))?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), Error> {
        self.out.write_all(b"]")?;
        self.out.flush()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HTML sink — a streamed <table>
// ---------------------------------------------------------------------------

/// [`RowSink`] that streams an HTML `<table>`: `<thead>` header on the
/// header call, one `<tbody>` `<tr>` per row, closing tags on finish.
/// Every header and cell is HTML-escaped.
pub(crate) struct HtmlSink {
    out: BufWriter<File>,
    sec: SecType,
    fmt: Vec<DataType>,
    data_idx: Vec<usize>,
    price_type_idx: Option<usize>,
    /// Reused row buffer to avoid per-row allocation.
    line: String,
}

impl HtmlSink {
    /// Creates an HTML sink writing to `path`.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` when `path` cannot be created.
    pub(crate) fn new(
        path: &Path,
        sec: SecType,
        fmt: Vec<DataType>,
        price_type_idx: Option<usize>,
    ) -> Result<Self, Error> {
        let data_idx = data_indices(&fmt, price_type_idx);
        let f = File::create(path)?;
        Ok(Self {
            out: BufWriter::with_capacity(1 << 20, f),
            sec,
            fmt,
            data_idx,
            price_type_idx,
            line: String::with_capacity(256),
        })
    }
}

impl RowSink for HtmlSink {
    fn write_header(&mut self) -> Result<(), Error> {
        let prefix: &[&str] = match self.sec {
            SecType::Option => &["symbol", "expiration", "strike", "right"],
            SecType::Stock | SecType::Index => &["symbol"],
        };
        self.line.clear();
        self.line.push_str("<table>\n<thead><tr>");
        for name in prefix {
            self.line.push_str("<th>");
            append_html_escaped(&mut self.line, name);
            self.line.push_str("</th>");
        }
        for &i in &self.data_idx {
            let name = self.fmt[i].name();
            self.line.push_str("<th>");
            append_html_escaped(&mut self.line, &name);
            self.line.push_str("</th>");
        }
        self.line.push_str("</tr></thead>\n<tbody>\n");
        self.out.write_all(self.line.as_bytes())?;
        Ok(())
    }

    fn write_row(&mut self, row: RowView<'_>) -> Result<(), Error> {
        use std::fmt::Write;
        self.line.clear();
        self.line.push_str("<tr>");
        match self.sec {
            SecType::Option => {
                // symbol may carry markup-significant bytes; the rest are
                // machine values with nothing to escape.
                self.line.push_str("<td>");
                append_html_escaped(&mut self.line, &row.entry.symbol);
                self.line.push_str("</td>");
                let _ = write!(self.line, "<td>{}</td>", row.entry.expiration.unwrap_or(0));
                let _ = write!(
                    self.line,
                    "<td>{}</td>",
                    row.entry.strike_dollars().unwrap_or(0.0)
                );
                let _ = write!(self.line, "<td>{}</td>", row.entry.right.unwrap_or('?'));
            }
            SecType::Stock | SecType::Index => {
                self.line.push_str("<td>");
                append_html_escaped(&mut self.line, &row.entry.symbol);
                self.line.push_str("</td>");
            }
        }
        let pt = price_type_for_row(row.data, self.price_type_idx);
        for &i in &self.data_idx {
            let val = row.data.get(i).copied().unwrap_or(0);
            self.line.push_str("<td>");
            if self.fmt[i].is_price() {
                if let Some(t) = pt {
                    fmt_price_into(&mut self.line, val, t)?;
                } else {
                    let _ = write!(self.line, "{val}");
                }
            } else {
                let _ = write!(self.line, "{val}");
            }
            self.line.push_str("</td>");
        }
        self.line.push_str("</tr>\n");
        self.out.write_all(self.line.as_bytes())?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), Error> {
        self.out.write_all(b"</tbody>\n</table>\n")?;
        self.out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("thetadatadx-flatfiles-writer-test-{pid}-{n}"))
    }

    #[test]
    fn data_indices_skips_price_type() {
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let idx = data_indices(&fmt, Some(2));
        assert_eq!(idx, vec![0, 1, 3]);
    }

    #[test]
    fn data_indices_no_price_type() {
        let fmt = vec![DataType::MsOfDay, DataType::OpenInterest, DataType::Date];
        let idx = data_indices(&fmt, None);
        assert_eq!(idx, vec![0, 1, 2]);
    }

    #[test]
    fn price_type_for_row_reads_column() {
        let row = vec![0, 0, 8, 0]; // PRICE_TYPE = 8 (cents)
        assert_eq!(price_type_for_row(&row, Some(2)), Some(8));
        assert_eq!(price_type_for_row(&row, None), None);
    }

    #[test]
    fn decode_price_uses_vendor_semantics() {
        // PRICE_TYPE = 8 means real = value * 0.01 (cents).
        assert!((decode_price(15025, 8).unwrap() - 150.25).abs() < 1e-9);
        // PRICE_TYPE = 10 means real = value (integer).
        assert!((decode_price(150, 10).unwrap() - 150.0).abs() < 1e-9);
        // PRICE_TYPE = 0 is the vendor "no price" sentinel.
        assert_eq!(decode_price(123, 0).unwrap(), 0.0);
        // Sub-cent micro-pricing: PRICE_TYPE = 4 => value * 1e-6.
        assert!((decode_price(19, 4).unwrap() - 1.9e-5).abs() < 1e-12);
    }

    #[test]
    fn fmt_price_preserves_full_precision() {
        let mut s = String::new();
        fmt_price_into(&mut s, 15025, 8).unwrap();
        assert_eq!(s, "150.25");
        s.clear();
        // Micro-priced option: must NOT round to 0.
        fmt_price_into(&mut s, 19, 4).unwrap();
        assert!(s.starts_with("0.0000") || s.contains("e-"), "got {s:?}");
        assert_ne!(s, "0.0000");
    }

    #[test]
    fn decode_price_rejects_price_type_above_max() {
        let err = decode_price(15_025, 20).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("price_type 20"),
            "error must capture the raw wire value (got {msg:?})"
        );
    }

    #[test]
    fn decode_price_rejects_negative_price_type() {
        let err = decode_price(99, -1).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("price_type -1"),
            "error must capture the raw wire value (got {msg:?})"
        );
    }

    #[test]
    fn decode_price_rejects_price_type_i32_max() {
        let err = decode_price(1, i32::MAX).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(&i32::MAX.to_string()),
            "error must capture the raw wire value (got {msg:?})"
        );
    }

    #[test]
    fn fmt_price_into_propagates_invalid_price_type() {
        let mut buf = String::from("preamble,");
        let res = fmt_price_into(&mut buf, 15_025, 20);
        assert!(res.is_err());
        // Reason: buffer must not retain a partial render after the error.
        assert_eq!(buf, "preamble,");
    }

    #[test]
    fn csv_sink_write_row_rejects_invalid_price_type() {
        let tmp = scratch_path();
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let mut sink = CsvSink::new(&tmp, SecType::Stock, fmt, Some(2)).unwrap();
        sink.write_header().unwrap();
        let entry = IndexEntry {
            symbol: "AAPL".into(),
            expiration: None,
            strike: None,
            right: None,
            block_start: 0,
            block_end: 0,
        };
        let row = [34_200_000_i32, 15_025, 20, 20_250_428];
        let res = sink.write_row(RowView {
            entry: &entry,
            data: &row,
        });
        assert!(res.is_err());
    }

    #[test]
    fn jsonl_sink_write_row_rejects_invalid_price_type() {
        let tmp = scratch_path();
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let mut sink = JsonlSink::new(&tmp, SecType::Stock, fmt, Some(2)).unwrap();
        sink.write_header().unwrap();
        let entry = IndexEntry {
            symbol: "AAPL".into(),
            expiration: None,
            strike: None,
            right: None,
            block_start: 0,
            block_end: 0,
        };
        let row = [34_200_000_i32, 15_025, 21, 20_250_428];
        let res = sink.write_row(RowView {
            entry: &entry,
            data: &row,
        });
        assert!(res.is_err());
    }

    #[test]
    fn flatfile_row_from_decoded_rejects_invalid_price_type() {
        use crate::flatfiles::decoded_row::FlatFileRow;
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let data_idx = vec![0_usize, 1, 3];
        let row = [34_200_000_i32, 15_025, 20, 20_250_428];
        let res =
            FlatFileRow::from_decoded("AAPL", None, None, None, &fmt, &row, &data_idx, Some(20));
        assert!(res.is_err());
    }

    /// Read back the strike value the CSV sink wrote for `entry`. CSV
    /// strike is the 3rd field: `symbol,expiration,strike,right,...`.
    fn csv_strike_for(entry: &IndexEntry, fmt: &[DataType], row_data: &[i32]) -> f64 {
        let path = scratch_path();
        let mut csv =
            CsvSink::new(&path, SecType::Option, fmt.to_vec(), Some(2)).expect("CsvSink::new");
        csv.write_header().expect("csv header");
        csv.write_row(RowView {
            entry,
            data: row_data,
        })
        .expect("csv row");
        Box::new(csv).finish().expect("csv finish");
        let text = std::fs::read_to_string(&path).expect("read csv");
        text.lines()
            .nth(1)
            .expect("csv data row")
            .split(',')
            .nth(2)
            .expect("csv strike field")
            .parse()
            .expect("csv strike parses as f64")
    }

    /// Read back the strike value the JSONL sink wrote for `entry`.
    fn jsonl_strike_for(entry: &IndexEntry, fmt: &[DataType], row_data: &[i32]) -> f64 {
        let path = scratch_path();
        let mut jsonl =
            JsonlSink::new(&path, SecType::Option, fmt.to_vec(), Some(2)).expect("JsonlSink::new");
        jsonl.write_header().expect("jsonl header");
        jsonl
            .write_row(RowView {
                entry,
                data: row_data,
            })
            .expect("jsonl row");
        Box::new(jsonl).finish().expect("jsonl finish");
        let text = std::fs::read_to_string(&path).expect("read jsonl");
        let parsed: serde_json::Value =
            serde_json::from_str(text.lines().next().expect("jsonl line")).expect("jsonl parses");
        parsed["strike"].as_f64().expect("jsonl strike is a number")
    }

    #[test]
    fn strike_is_dollars_and_identical_across_csv_and_jsonl() {
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let row_data = [34_200_000_i32, 15_025, 8, 20_250_428];
        // Whole-dollar and sub-dollar wire strikes (tenths of a cent).
        // 580000 -> $580.00, 1500 -> $1.50.
        for (wire_strike, expected_dollars) in [(580_000_i32, 580.0_f64), (1_500_i32, 1.5_f64)] {
            let entry = IndexEntry {
                symbol: "SPX".into(),
                expiration: Some(20_260_516),
                strike: Some(wire_strike),
                right: Some('C'),
                block_start: 0,
                block_end: 0,
            };
            let csv_strike = csv_strike_for(&entry, &fmt, &row_data);
            let jsonl_strike = jsonl_strike_for(&entry, &fmt, &row_data);
            assert!(
                (csv_strike - expected_dollars).abs() < 1e-9,
                "CSV strike must be dollars: got {csv_strike}, want {expected_dollars}"
            );
            assert!(
                (jsonl_strike - expected_dollars).abs() < 1e-9,
                "JSONL strike must be dollars: got {jsonl_strike}, want {expected_dollars}"
            );
            assert_eq!(
                csv_strike, jsonl_strike,
                "CSV and JSONL strike must be identical"
            );
        }
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn strike_is_dollars_and_identical_across_csv_jsonl_arrow() {
        use crate::flatfiles::arrow::rows_to_arrow;
        use crate::flatfiles::decoded_row::FlatFileRow;
        use arrow_array::cast::AsArray;
        use arrow_array::types::Float64Type;

        // Schema with a PRICE_TYPE column so every surface decodes prices
        // the same way; the strike assertion is independent of price.
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let data_idx = data_indices(&fmt, Some(2));
        let row_data = [34_200_000_i32, 15_025, 8, 20_250_428];

        // Whole-dollar and sub-dollar wire strikes (tenths of a cent).
        // 580000 -> $580.00, 1500 -> $1.50.
        for (wire_strike, expected_dollars) in [(580_000_i32, 580.0_f64), (1_500_i32, 1.5_f64)] {
            let entry = IndexEntry {
                symbol: "SPX".into(),
                expiration: Some(20_260_516),
                strike: Some(wire_strike),
                right: Some('C'),
                block_start: 0,
                block_end: 0,
            };

            let csv_strike = csv_strike_for(&entry, &fmt, &row_data);
            let jsonl_strike = jsonl_strike_for(&entry, &fmt, &row_data);

            // Arrow: strike column is Float64 dollars.
            let typed_row = FlatFileRow::from_decoded(
                &entry.symbol,
                entry.expiration,
                entry.strike,
                entry.right,
                &fmt,
                &row_data,
                &data_idx,
                Some(row_data[2]),
            )
            .expect("from_decoded");
            let batch = rows_to_arrow(std::slice::from_ref(&typed_row)).expect("rows_to_arrow");
            let strike_col = batch
                .column_by_name("strike")
                .expect("strike column")
                .as_primitive::<Float64Type>();
            let arrow_strike = strike_col.value(0);
            assert!(
                (arrow_strike - expected_dollars).abs() < 1e-9,
                "Arrow strike must be dollars: got {arrow_strike}, want {expected_dollars}"
            );

            // The load-bearing invariant: all three surfaces agree exactly.
            assert_eq!(
                csv_strike, jsonl_strike,
                "CSV and JSONL strike must be identical"
            );
            assert_eq!(
                csv_strike, arrow_strike,
                "CSV and Arrow strike must be identical"
            );
        }
    }

    #[test]
    fn csv_sink_write_row_smoke_with_in_range_price_type() {
        let tmp = scratch_path();
        let fmt = vec![
            DataType::MsOfDay,
            DataType::Bid,
            DataType::PriceType,
            DataType::Date,
        ];
        let mut sink = CsvSink::new(&tmp, SecType::Stock, fmt, Some(2)).expect("CsvSink::new");
        sink.write_header().expect("write_header");
        let entry = IndexEntry {
            symbol: "AAPL".into(),
            expiration: None,
            strike: None,
            right: None,
            block_start: 0,
            block_end: 0,
        };
        let row = [34_200_000_i32, 15_025, 8, 20_250_428];
        sink.write_row(RowView {
            entry: &entry,
            data: &row,
        })
        .expect("in-range PRICE_TYPE row must serialize");
        Box::new(sink).finish().expect("finish");
        let contents = std::fs::read_to_string(&tmp).expect("read_to_string");
        assert!(contents.contains("150.25"));
    }

    #[test]
    fn json_array_sink_streams_valid_array() {
        let tmp = scratch_path();
        let fmt = vec![DataType::MsOfDay, DataType::Bid, DataType::PriceType];
        let mut sink = JsonArraySink::new(&tmp, SecType::Stock, fmt, Some(2)).expect("new");
        sink.write_header().expect("header");
        let entry = IndexEntry {
            symbol: "AAPL".into(),
            expiration: None,
            strike: None,
            right: None,
            block_start: 0,
            block_end: 0,
        };
        for bid in [15_025_i32, 20_050] {
            sink.write_row(RowView {
                entry: &entry,
                data: &[34_200_000, bid, 8],
            })
            .expect("row");
        }
        Box::new(sink).finish().expect("finish");
        let text = std::fs::read_to_string(&tmp).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON array");
        let arr = parsed.as_array().expect("top level is an array");
        assert_eq!(arr.len(), 2, "one element per row");
        assert!((arr[0]["bid"].as_f64().unwrap() - 150.25).abs() < 1e-9);
        assert_eq!(arr[0]["symbol"].as_str(), Some("AAPL"));
    }

    #[test]
    fn json_array_sink_empty_is_bracket_pair() {
        let tmp = scratch_path();
        let fmt = vec![DataType::MsOfDay, DataType::Bid, DataType::PriceType];
        let mut sink = JsonArraySink::new(&tmp, SecType::Stock, fmt, Some(2)).expect("new");
        sink.write_header().expect("header");
        Box::new(sink).finish().expect("finish");
        assert_eq!(std::fs::read_to_string(&tmp).expect("read"), "[]");
    }

    #[test]
    fn html_sink_escapes_and_frames_table() {
        let tmp = scratch_path();
        let fmt = vec![DataType::MsOfDay, DataType::Bid, DataType::PriceType];
        let mut sink = HtmlSink::new(&tmp, SecType::Stock, fmt, Some(2)).expect("new");
        sink.write_header().expect("header");
        // A markup-significant symbol must be escaped in the cell.
        let entry = IndexEntry {
            symbol: "A<B&C".into(),
            expiration: None,
            strike: None,
            right: None,
            block_start: 0,
            block_end: 0,
        };
        sink.write_row(RowView {
            entry: &entry,
            data: &[34_200_000, 15_025, 8],
        })
        .expect("row");
        Box::new(sink).finish().expect("finish");
        let html = std::fs::read_to_string(&tmp).expect("read");
        assert!(
            html.starts_with("<table>\n<thead><tr>"),
            "table opens: {html}"
        );
        assert!(
            html.ends_with("</tbody>\n</table>\n"),
            "table closes: {html}"
        );
        assert!(html.contains("<th>symbol</th>"), "header cell: {html}");
        assert!(
            html.contains("<td>A&lt;B&amp;C</td>"),
            "cell must be HTML-escaped: {html}"
        );
        assert!(html.contains("<td>150.25</td>"), "price decoded: {html}");
    }
}
