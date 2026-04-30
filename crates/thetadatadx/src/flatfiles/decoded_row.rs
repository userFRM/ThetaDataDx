//! Typed in-memory row for the FLATFILES surface.
//!
//! Returned by [`crate::flatfiles::flatfile_request_decoded`] and the
//! corresponding [`crate::ThetaDataDx`] convenience methods. Callers
//! that want to drive an algorithm against the full universe for a
//! given `(sec, req, date)` tuple use this entry point instead of
//! writing a CSV / JSONL file and parsing it back in.
//!
//! The row carries the contract key the vendor prepends to every CSV
//! row, plus the per-data-type column values keyed by the original
//! (lowercase) column name. Price columns are pre-divided by the row's
//! `PRICE_TYPE` exponent so the caller never has to apply the divisor
//! themselves.

use crate::flatfiles::datatype::DataType;

/// Single decoded flat-file row.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatFileRow {
    /// Underlying root symbol (e.g. `"SPY"`).
    pub root: String,
    /// Expiration in `YYYYMMDD`. `None` for stock blobs.
    pub expiration: Option<i32>,
    /// Strike in vendor units (1/1000 of a dollar). `None` for stocks.
    pub strike: Option<i32>,
    /// `'C'` (call), `'P'` (put), or `None` for stocks / unknown.
    pub right: Option<char>,
    /// One entry per non-PRICE_TYPE schema column, in vendor order.
    /// `(column_name, value)` — column name is lowercase to match the
    /// vendor CSV header.
    pub fields: Vec<(String, FlatFileValue)>,
}

/// Cell value in a decoded flat-file row.
#[derive(Debug, Clone, PartialEq)]
pub enum FlatFileValue {
    /// Plain integer column (counts, ms-of-day, dates, ordinals).
    Int(i32),
    /// Price column already divided by the row's `PRICE_TYPE` exponent.
    Price(f64),
}

impl FlatFileRow {
    /// Build a row from the decoded data slice plus the schema. Callers
    /// at the decode layer use this to avoid open-coding the column
    /// projection in two places.
    pub(crate) fn from_decoded(
        root: &str,
        expiration: Option<i32>,
        strike: Option<i32>,
        right: Option<char>,
        fmt: &[DataType],
        data: &[i32],
        data_idx: &[usize],
        divisor: Option<f64>,
    ) -> Self {
        let mut fields = Vec::with_capacity(data_idx.len());
        for &i in data_idx {
            let val = data.get(i).copied().unwrap_or(0);
            let dt = fmt[i];
            let cell = if dt.is_price() {
                let d = divisor.unwrap_or(1.0);
                FlatFileValue::Price((val as f64) / d)
            } else {
                FlatFileValue::Int(val)
            };
            fields.push((dt.name().into_owned(), cell));
        }
        Self {
            root: root.to_string(),
            expiration,
            strike,
            right,
            fields,
        }
    }
}
