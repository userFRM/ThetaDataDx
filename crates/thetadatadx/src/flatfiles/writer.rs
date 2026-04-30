//! Format-pluggable row writer for the FLATFILES surface.
//!
//! Each output format implements [`RowSink`], a tiny three-method
//! interface: write the header once, accept rows one-by-one with the
//! contract key + decoded data fields, and finalize on completion. The
//! decoder layer (see `decoded.rs`) drives a single sink regardless of
//! format — CSV or JSONL.
//!
//! The contract key passed to each sink call carries the
//! `(root, exp, strike, right)` columns the vendor prepends to every CSV
//! row. For stock entries, `exp / strike / right` are `None` and only
//! `root` is written.
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

/// Per-row price divisor.
///
/// When PRICE_TYPE is in the schema, the value at that column is the
/// exponent N, so the price = `int_value / 10^N`. With no PRICE_TYPE
/// column the multiplier is 1 (i.e. the integer IS the price already, in
/// vendor convention) — the vendor's `toCSV2` does not call
/// `PriceCalc.fmtPrice` in that branch, so emitting the integer
/// unchanged matches the reference output.
pub(crate) fn price_divisor(row: &[i32], price_type_idx: Option<usize>) -> Option<f64> {
    price_type_idx.map(|idx| {
        let n = row.get(idx).copied().unwrap_or(0);
        let n = n.clamp(0, 18) as u32;
        10f64.powi(n as i32)
    })
}

/// Format a price value in vendor style: divide by `10^N`, render with
/// 4 fractional digits and **no trailing-zero stripping** (the vendor's
/// `PriceCalc.fmtPrice(builder, value, 4)` is fixed-precision).
pub(crate) fn fmt_price_into(buf: &mut String, integer: i32, divisor: f64) {
    use std::fmt::Write;
    let v = (integer as f64) / divisor;
    // Match vendor: 4 decimals, no exponent, no thousands separator.
    let _ = write!(buf, "{v:.4}");
}

/// Build the contract-prefix segment of a CSV row.
fn append_csv_prefix(buf: &mut String, entry: &IndexEntry, sec: SecType) {
    use std::fmt::Write;
    match sec {
        SecType::Option | SecType::Index => {
            buf.push_str(&entry.root);
            buf.push(',');
            let _ = write!(buf, "{}", entry.exp.unwrap_or(0));
            buf.push(',');
            let _ = write!(buf, "{}", entry.strike.unwrap_or(0));
            buf.push(',');
            buf.push(entry.right.unwrap_or('?'));
            buf.push(',');
        }
        SecType::Stock => {
            buf.push_str(&entry.root);
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
            SecType::Option | SecType::Index => self.line.push_str("root,expiration,strike,right,"),
            SecType::Stock => self.line.push_str("root,"),
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
        let divisor = price_divisor(row.data, self.price_type_idx);
        for (n, &i) in self.data_idx.iter().enumerate() {
            if n > 0 {
                self.line.push(',');
            }
            let val = row.data.get(i).copied().unwrap_or(0);
            if self.fmt[i].is_price() {
                if let Some(d) = divisor {
                    fmt_price_into(&mut self.line, val, d);
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
                    "root".into(),
                    serde_json::Value::String(row.entry.root.clone()),
                );
                obj.insert(
                    "expiration".into(),
                    serde_json::Value::Number(row.entry.exp.unwrap_or(0).into()),
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
                    "root".into(),
                    serde_json::Value::String(row.entry.root.clone()),
                );
            }
        }
        let divisor = price_divisor(row.data, self.price_type_idx);
        for &i in &self.data_idx {
            let val = row.data.get(i).copied().unwrap_or(0);
            let key = self.fmt[i].name().into_owned();
            let v = if self.fmt[i].is_price() {
                if let Some(d) = divisor {
                    let f = (val as f64) / d;
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
    fn price_divisor_clamped() {
        let row = vec![0, 0, 4, 0]; // PRICE_TYPE = 4
        let d = price_divisor(&row, Some(2)).unwrap();
        assert!((d - 10_000f64).abs() < 1e-9);
    }

    #[test]
    fn price_divisor_negative_clamped_to_zero() {
        let row = vec![-1];
        let d = price_divisor(&row, Some(0)).unwrap();
        assert!((d - 1.0).abs() < 1e-9);
    }

    #[test]
    fn fmt_price_four_decimals() {
        let mut s = String::new();
        fmt_price_into(&mut s, 15025, 10_000.0);
        assert_eq!(s, "1.5025");
        s.clear();
        fmt_price_into(&mut s, 0, 10_000.0);
        assert_eq!(s, "0.0000");
    }
}
