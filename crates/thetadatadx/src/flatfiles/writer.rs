//! Format-pluggable row writer for the FLATFILES surface.
//!
//! Each output format implements [`RowSink`], a tiny three-method
//! interface: write the header once, accept rows one-by-one with the
//! contract key + decoded data fields, and finalize on completion. The
//! decoder layer (see `decoded.rs`) drives a single sink regardless of
//! format — CSV or JSONL.
//!
//! The contract key passed to each sink call carries the
//! `(symbol, expiration, strike, right)` columns prepended to every CSV
//! row, matching the v3 vendor surface. For stock entries, `expiration /
//! strike / right` are `None` and only `symbol` is written. See:
//! <https://docs.thetadata.us/Articles/Getting-Started/v2-migration-guide.html#_5-parameter-mapping>.
//!
//! Both sinks must produce the **same logical rows**; only the on-disk
//! encoding differs. This is what makes the byte-match test on `Csv` a
//! sufficient proxy for verifying `Jsonl`.

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
    fn write_header(&mut self) -> Result<(), Error>;
    fn write_row(&mut self, row: RowView<'_>) -> Result<(), Error>;
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

/// Per-row PRICE_TYPE exponent for `tdbe::Price` decoding.
///
/// When PRICE_TYPE is in the schema, the value at that column is the
/// vendor `price_type` field (real price = `value * 10^(price_type - 10)`).
/// When PRICE_TYPE is absent, the column is not a price (the vendor's
/// `toCSV2` does not call `PriceCalc.fmtPrice` in that branch), so the
/// integer value is emitted unchanged.
pub(crate) fn price_type_for_row(row: &[i32], price_type_idx: Option<usize>) -> Option<i32> {
    price_type_idx.map(|idx| row.get(idx).copied().unwrap_or(0))
}

/// Convert a wire `(value, price_type)` pair to its real f64 price using
/// the canonical [`tdbe::types::price::Price`] semantics. Returns 0.0 for
/// `price_type == 0` (vendor sentinel for "no price").
pub(crate) fn decode_price(integer: i32, price_type: i32) -> f64 {
    tdbe::types::price::Price::new(integer, price_type).to_f64()
}

/// Render the decoded price using Rust's default `f64` Display, which
/// preserves the full IEEE-754 precision the wire decoder produced. For
/// micro-priced contracts (sub-cent options) this is the only viable
/// representation — fixed-point rendering rounds those to zero.
pub(crate) fn fmt_price_into(buf: &mut String, integer: i32, price_type: i32) {
    use std::fmt::Write;
    let v = decode_price(integer, price_type);
    let _ = write!(buf, "{v}");
}

/// Build the contract-prefix segment of a CSV row.
fn append_csv_prefix(buf: &mut String, entry: &IndexEntry, sec: SecType) {
    use std::fmt::Write;
    match sec {
        SecType::Option | SecType::Index => {
            buf.push_str(&entry.symbol);
            buf.push(',');
            let _ = write!(buf, "{}", entry.expiration.unwrap_or(0));
            buf.push(',');
            let _ = write!(buf, "{}", entry.strike.unwrap_or(0));
            buf.push(',');
            buf.push(entry.right.unwrap_or('?'));
            buf.push(',');
        }
        SecType::Stock => {
            buf.push_str(&entry.symbol);
            buf.push(',');
        }
    }
}

// ---------------------------------------------------------------------------
// CSV sink — vendor byte format
// ---------------------------------------------------------------------------

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
            SecType::Option | SecType::Index => {
                self.line.push_str("symbol,expiration,strike,right,");
            }
            SecType::Stock => self.line.push_str("symbol,"),
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
                    fmt_price_into(&mut self.line, val, t);
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

pub(crate) struct JsonlSink {
    out: BufWriter<File>,
    sec: SecType,
    fmt: Vec<DataType>,
    data_idx: Vec<usize>,
    price_type_idx: Option<usize>,
}

impl JsonlSink {
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
        let mut obj = serde_json::Map::with_capacity(self.data_idx.len() + 4);
        match self.sec {
            SecType::Option | SecType::Index => {
                obj.insert(
                    "symbol".into(),
                    serde_json::Value::String(row.entry.symbol.clone()),
                );
                obj.insert(
                    "expiration".into(),
                    serde_json::Value::Number(row.entry.expiration.unwrap_or(0).into()),
                );
                obj.insert(
                    "strike".into(),
                    serde_json::Value::Number(row.entry.strike.unwrap_or(0).into()),
                );
                obj.insert(
                    "right".into(),
                    serde_json::Value::String(row.entry.right.unwrap_or('?').to_string()),
                );
            }
            SecType::Stock => {
                obj.insert(
                    "symbol".into(),
                    serde_json::Value::String(row.entry.symbol.clone()),
                );
            }
        }
        let pt = price_type_for_row(row.data, self.price_type_idx);
        for &i in &self.data_idx {
            let val = row.data.get(i).copied().unwrap_or(0);
            let key = self.fmt[i].name().into_owned();
            let v = if self.fmt[i].is_price() {
                if let Some(t) = pt {
                    let f = decode_price(val, t);
                    serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Number(val.into())
                }
            } else {
                serde_json::Value::Number(val.into())
            };
            obj.insert(key, v);
        }
        serde_json::to_writer(&mut self.out, &serde_json::Value::Object(obj))
            .map_err(|e| Error::Config(format!("flatfiles: jsonl encode failed: {e}")))?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), Error> {
        self.out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!((decode_price(15025, 8) - 150.25).abs() < 1e-9);
        // PRICE_TYPE = 10 means real = value (integer).
        assert!((decode_price(150, 10) - 150.0).abs() < 1e-9);
        // PRICE_TYPE = 0 is the vendor "no price" sentinel.
        assert_eq!(decode_price(123, 0), 0.0);
        // Sub-cent micro-pricing: PRICE_TYPE = 4 => value * 1e-6.
        assert!((decode_price(19, 4) - 1.9e-5).abs() < 1e-12);
    }

    #[test]
    fn fmt_price_preserves_full_precision() {
        let mut s = String::new();
        fmt_price_into(&mut s, 15025, 8);
        assert_eq!(s, "150.25");
        s.clear();
        // Micro-priced option: must NOT round to 0.
        fmt_price_into(&mut s, 19, 4);
        assert!(s.starts_with("0.0000") || s.contains("e-"), "got {s:?}");
        assert_ne!(s, "0.0000");
    }
}
