//! Dynamic-schema Arrow conversion for [`FlatFileRow`] collections.
//!
//! Unlike the per-tick Arrow conversions, the FLATFILES
//! schema is determined at request time by `(SecType, ReqType)` —
//! `FlatFileRow::fields` carries `Vec<(String, FlatFileValue)>` whose
//! shape only becomes known after the vendor blob is decoded. This
//! module walks the first row to infer column names + Arrow `DataType`,
//! pre-allocates one builder per column, and appends every row's values
//! in one pass.
//!
//! Single-source-of-truth dispatch: every binding (Python, TypeScript,
//! C++) that wants Arrow output funnels through [`crate::flatfiles::arrow::rows_to_arrow`].
//! The mapping `FlatFileValue -> arrow_schema::DataType` lives once in
//! [`crate::flatfiles::arrow::flatfile_value_arrow_type`] and is reused
//! both at schema-inference time and at builder-finalization time.
//!
//! # Schema rules
//!
//! - The first three columns are the contract key the vendor prepends
//!   to every row: `symbol` (Utf8), `expiration` (Int32, nullable for
//!   stocks), `strike` (Int32, nullable for stocks), `right` (Utf8,
//!   nullable for stocks). Storing the right as a single-character
//!   string instead of [`arrow_schema::DataType::Dictionary`] keeps the
//!   schema portable across Python / TS / C++ Arrow IPC consumers.
//! - One column per entry in [`FlatFileRow::fields`], in vendor order.
//!   The column name comes from the field's first tuple slot;
//!   [`FlatFileValue::Int`] -> Int32, [`FlatFileValue::Price`] -> Float64.
//! - Subsequent rows must match the inferred schema 1:1 (same column
//!   count, same names in the same order, same value variant per
//!   column). Mismatches return [`crate::error::Error::Decode`] — the
//!   caller has decoded a heterogenous batch which violates the
//!   FLATFILES per-`(SecType, ReqType)` invariant.
//!
//! # Empty input
//!
//! Calling `rows_to_arrow(&[])` returns an `arrow_array::RecordBatch`
//! with zero columns and zero rows. The caller cannot recover a schema from this
//! — they must inspect their own request to know what shape was
//! expected. This is consistent with how empty MDDS responses surface
//! through the Python `<TickName>List` wrappers.

use std::sync::Arc;

use arrow_array::builder::{Float64Builder, Int32Builder, StringBuilder};
use arrow_array::{Array, RecordBatch, RecordBatchOptions};
use arrow_schema::{DataType, Field, Schema};

use crate::error::Error;
use crate::flatfiles::decoded_row::{FlatFileRow, FlatFileValue};

/// Arrow `DataType` for a [`FlatFileValue`] variant.
///
/// SSOT mapping reused at schema-inference time and at column-finalize
/// time inside [`rows_to_arrow`]. Bindings that need their own column
/// dispatch (e.g. emitting a polars `Series` per column) should call
/// this rather than re-derive the mapping.
#[must_use]
pub fn flatfile_value_arrow_type(value: &FlatFileValue) -> DataType {
    match value {
        FlatFileValue::Int(_) => DataType::Int32,
        FlatFileValue::Price(_) => DataType::Float64,
    }
}

/// Convert a slice of [`FlatFileRow`] into an Arrow [`RecordBatch`].
///
/// Schema is inferred from the first row's contract-key columns plus
/// its `fields` vector (see module docs). Subsequent rows must match
/// the inferred schema; any deviation returns [`Error::Decode`].
///
/// # Errors
///
/// Returns [`Error::Decode`] when:
/// - A later row has a different `fields.len()` than the first row;
/// - A later row has a column name in a different position;
/// - A later row's value variant does not match the inferred Arrow
///   `DataType` for that column;
/// - Arrow's internal `RecordBatch::try_new` rejects the assembled
///   columns (e.g. unequal length — defensive, the per-row append loop
///   guarantees equal length).
pub fn rows_to_arrow(rows: &[FlatFileRow]) -> Result<RecordBatch, Error> {
    if rows.is_empty() {
        let schema = Arc::new(Schema::empty());
        // Empty schema + zero rows requires the explicit row-count
        // option; `try_new` rejects the otherwise-ambiguous shape.
        let options = RecordBatchOptions::new().with_row_count(Some(0));
        return RecordBatch::try_new_with_options(schema, Vec::new(), &options)
            .map_err(|err| Error::decode_arrow(format!("rows_to_arrow: {err}")));
    }

    let first = &rows[0];
    let column_count = 4 + first.fields.len();

    let mut symbol_builder = StringBuilder::with_capacity(rows.len(), rows.len() * 8);
    let mut expiration_builder = Int32Builder::with_capacity(rows.len());
    let mut strike_builder = Int32Builder::with_capacity(rows.len());
    let mut right_builder = StringBuilder::with_capacity(rows.len(), rows.len());

    enum DataBuilder {
        Int(Int32Builder),
        Price(Float64Builder),
    }
    impl DataBuilder {
        fn data_type(&self) -> DataType {
            match self {
                Self::Int(_) => DataType::Int32,
                Self::Price(_) => DataType::Float64,
            }
        }
    }

    let mut data_builders: Vec<(String, DataBuilder)> = first
        .fields
        .iter()
        .map(|(name, value)| {
            let builder = match value {
                FlatFileValue::Int(_) => DataBuilder::Int(Int32Builder::with_capacity(rows.len())),
                FlatFileValue::Price(_) => {
                    DataBuilder::Price(Float64Builder::with_capacity(rows.len()))
                }
            };
            (name.clone(), builder)
        })
        .collect();

    for (row_idx, row) in rows.iter().enumerate() {
        symbol_builder.append_value(&row.symbol);
        match row.expiration {
            Some(v) => expiration_builder.append_value(v),
            None => expiration_builder.append_null(),
        }
        match row.strike {
            Some(v) => strike_builder.append_value(v),
            None => strike_builder.append_null(),
        }
        match row.right {
            Some(c) => {
                let mut buf = [0u8; 4];
                right_builder.append_value(c.encode_utf8(&mut buf));
            }
            None => right_builder.append_null(),
        }

        if row.fields.len() != data_builders.len() {
            return Err(Error::decode_truncated_row(
                row_idx,
                data_builders.len(),
                row.fields.len(),
            ));
        }
        for (col_idx, ((expected_name, builder), (actual_name, value))) in
            data_builders.iter_mut().zip(row.fields.iter()).enumerate()
        {
            if expected_name != actual_name {
                return Err(Error::decode_column_type_mismatch(
                    row_idx,
                    format!("col_{col_idx}"),
                    expected_name.clone(),
                    actual_name.clone(),
                ));
            }
            match (builder, value) {
                (DataBuilder::Int(b), FlatFileValue::Int(v)) => b.append_value(*v),
                (DataBuilder::Price(b), FlatFileValue::Price(v)) => b.append_value(*v),
                (b, v) => {
                    return Err(Error::decode_column_type_mismatch(
                        row_idx,
                        actual_name,
                        format!("{:?}", b.data_type()),
                        format!("{:?}", flatfile_value_arrow_type(v)),
                    ));
                }
            }
        }
    }

    let mut fields: Vec<Field> = Vec::with_capacity(column_count);
    fields.push(Field::new("symbol", DataType::Utf8, false));
    fields.push(Field::new("expiration", DataType::Int32, true));
    fields.push(Field::new("strike", DataType::Int32, true));
    fields.push(Field::new("right", DataType::Utf8, true));

    let mut columns: Vec<Arc<dyn Array>> = Vec::with_capacity(column_count);
    columns.push(Arc::new(symbol_builder.finish()));
    columns.push(Arc::new(expiration_builder.finish()));
    columns.push(Arc::new(strike_builder.finish()));
    columns.push(Arc::new(right_builder.finish()));

    for (name, builder) in data_builders {
        let dtype = builder.data_type();
        let array: Arc<dyn Array> = match builder {
            DataBuilder::Int(mut b) => Arc::new(b.finish()),
            DataBuilder::Price(mut b) => Arc::new(b.finish()),
        };
        fields.push(Field::new(name, dtype, false));
        columns.push(array);
    }

    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, columns)
        .map_err(|err| Error::decode_arrow(format!("rows_to_arrow: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        symbol: &str,
        expiration: Option<i32>,
        strike: Option<i32>,
        right: Option<char>,
        fields: Vec<(&str, FlatFileValue)>,
    ) -> FlatFileRow {
        FlatFileRow {
            symbol: symbol.to_string(),
            expiration,
            strike,
            right,
            fields: fields
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn empty_input_yields_empty_batch() {
        let batch = rows_to_arrow(&[]).expect("empty input should succeed");
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.num_columns(), 0);
    }

    #[test]
    fn single_row_stock_no_optional_keys() {
        let rows = vec![row(
            "SPY",
            None,
            None,
            None,
            vec![
                ("ms_of_day", FlatFileValue::Int(34_200_000)),
                ("price", FlatFileValue::Price(450.25)),
                ("size", FlatFileValue::Int(100)),
            ],
        )];
        let batch = rows_to_arrow(&rows).expect("single row should succeed");
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 7);
        let schema = batch.schema();
        assert_eq!(schema.field(0).name(), "symbol");
        assert_eq!(schema.field(1).name(), "expiration");
        assert!(schema.field(1).is_nullable());
        assert_eq!(schema.field(4).name(), "ms_of_day");
        assert_eq!(schema.field(4).data_type(), &DataType::Int32);
        assert_eq!(schema.field(5).name(), "price");
        assert_eq!(schema.field(5).data_type(), &DataType::Float64);
    }

    #[test]
    fn multi_row_option_with_mixed_types() {
        let rows = vec![
            row(
                "SPY",
                Some(20_260_516),
                Some(450_000),
                Some('C'),
                vec![
                    ("ms_of_day", FlatFileValue::Int(34_200_000)),
                    ("bid", FlatFileValue::Price(1.25)),
                    ("ask", FlatFileValue::Price(1.30)),
                ],
            ),
            row(
                "SPY",
                Some(20_260_516),
                Some(450_000),
                Some('C'),
                vec![
                    ("ms_of_day", FlatFileValue::Int(34_200_500)),
                    ("bid", FlatFileValue::Price(1.26)),
                    ("ask", FlatFileValue::Price(1.31)),
                ],
            ),
        ];
        let batch = rows_to_arrow(&rows).expect("multi-row should succeed");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 7);
    }

    #[test]
    fn schema_mismatch_field_count_errors() {
        let rows = vec![
            row(
                "SPY",
                None,
                None,
                None,
                vec![("a", FlatFileValue::Int(1)), ("b", FlatFileValue::Int(2))],
            ),
            row("SPY", None, None, None, vec![("a", FlatFileValue::Int(1))]),
        ];
        let err = rows_to_arrow(&rows).expect_err("mismatched field count must error");
        match err {
            Error::Decode {
                kind:
                    crate::error::DecodeErrorKind::TruncatedRow {
                        row_idx,
                        expected_columns,
                        actual_columns,
                    },
                ..
            } => {
                assert_eq!(row_idx, 1);
                assert_eq!(expected_columns, 2);
                assert_eq!(actual_columns, 1);
            }
            other => panic!("expected Error::Decode TruncatedRow, got {other:?}"),
        }
    }

    #[test]
    fn schema_mismatch_field_name_errors() {
        let rows = vec![
            row("SPY", None, None, None, vec![("a", FlatFileValue::Int(1))]),
            row("SPY", None, None, None, vec![("b", FlatFileValue::Int(1))]),
        ];
        let err = rows_to_arrow(&rows).expect_err("renamed column must error");
        match err {
            Error::Decode {
                kind:
                    crate::error::DecodeErrorKind::ColumnTypeMismatch {
                        expected, actual, ..
                    },
                ..
            } => {
                assert_eq!(expected, "a");
                assert_eq!(actual, "b");
            }
            other => panic!("expected Error::Decode ColumnTypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn schema_mismatch_value_variant_errors() {
        let rows = vec![
            row("SPY", None, None, None, vec![("a", FlatFileValue::Int(1))]),
            row(
                "SPY",
                None,
                None,
                None,
                vec![("a", FlatFileValue::Price(1.0))],
            ),
        ];
        let err = rows_to_arrow(&rows).expect_err("variant change must error");
        match err {
            Error::Decode {
                kind: crate::error::DecodeErrorKind::ColumnTypeMismatch { column_name, .. },
                ..
            } => {
                assert_eq!(column_name, "a");
            }
            other => panic!("expected Error::Decode ColumnTypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn flatfile_value_arrow_type_dispatch() {
        assert_eq!(
            flatfile_value_arrow_type(&FlatFileValue::Int(0)),
            DataType::Int32
        );
        assert_eq!(
            flatfile_value_arrow_type(&FlatFileValue::Price(0.0)),
            DataType::Float64
        );
    }
}
