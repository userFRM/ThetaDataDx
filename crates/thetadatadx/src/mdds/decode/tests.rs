//! Tests for the row-cell decoders, column extractors, and v3 hand-written
//! parsers. Eastern-time / DST primitive tests live with their canonical
//! home in `tdbe::time`.

use super::cell::{
    row_number, row_number_i64, row_price_f64, row_price_type, row_price_value, row_text,
};
use super::dual_type_columns::{
    parse_calendar_days_v3, parse_iso_date, parse_option_contracts_v3, parse_time_text,
};
use super::dual_type_columns::{
    CALENDAR_STATUS_EARLY_CLOSE, CALENDAR_STATUS_FULL_CLOSE, CALENDAR_STATUS_OPEN,
    CALENDAR_STATUS_WEEKEND,
};
use super::{
    extract_number_column, parse_eod_ticks, parse_greeks_all_ticks, parse_greeks_first_order_ticks,
    parse_greeks_second_order_ticks, parse_greeks_third_order_ticks, parse_quote_ticks,
    parse_trade_ticks, DecodeError,
};
use crate::proto;

/// Build a DataValue containing a Number.
fn dv_number(n: i64) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Number(n)),
    }
}

/// Build a DataValue containing a Price.
fn dv_price(value: i32, r#type: i32) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Price(proto::Price {
            value,
            r#type,
        })),
    }
}

/// Build a DataValue containing NullValue.
fn dv_null() -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::NullValue(0)),
    }
}

/// Build a DataValue containing a Timestamp.
fn dv_timestamp(epoch_ms: u64) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Timestamp(
            proto::ZonedDateTime { epoch_ms, zone: 0 },
        )),
    }
}

/// Build a DataValue with no data_type set (missing).
fn dv_missing() -> proto::DataValue {
    proto::DataValue { data_type: None }
}

/// Build a DataValue containing Text.
fn dv_text(s: &str) -> proto::DataValue {
    proto::DataValue {
        data_type: Some(proto::data_value::DataType::Text(s.to_string())),
    }
}

fn row_of(values: Vec<proto::DataValue>) -> proto::DataValueList {
    proto::DataValueList { values }
}

#[test]
fn row_number_returns_value_for_number_cell() {
    let row = row_of(vec![dv_number(42)]);
    assert_eq!(row_number(&row, 0).unwrap(), Some(42));
}

#[test]
fn row_number_returns_none_for_null_cell() {
    let row = row_of(vec![dv_null()]);
    assert_eq!(row_number(&row, 0).unwrap(), None);
}

#[test]
fn row_number_errors_on_unset_cell() {
    // A DataValue with the oneof unset is a wire-protocol anomaly.
    // The upstream parser hits the default arm for `DATATYPE_NOT_SET`
    // and throws; we surface it as `TypeMismatch { observed: "Unset" }`.
    let row = row_of(vec![dv_missing()]);
    assert_eq!(
        row_number(&row, 0),
        Err(DecodeError::TypeMismatch {
            column: 0,
            expected: "Number|Timestamp",
            observed: "Unset",
        })
    );
}

#[test]
fn row_number_errors_on_out_of_bounds() {
    let row = row_of(vec![]);
    assert_eq!(
        row_number(&row, 5),
        Err(DecodeError::MissingCell { column: 5 })
    );
}

#[test]
fn row_number_errors_on_text_cell() {
    let row = row_of(vec![dv_text("oops")]);
    assert_eq!(
        row_number(&row, 0),
        Err(DecodeError::TypeMismatch {
            column: 0,
            expected: "Number|Timestamp",
            observed: "Text",
        })
    );
}

#[test]
fn row_number_errors_on_price_cell() {
    let row = row_of(vec![dv_price(12345, 10)]);
    assert_eq!(
        row_number(&row, 0),
        Err(DecodeError::TypeMismatch {
            column: 0,
            expected: "Number|Timestamp",
            observed: "Price",
        })
    );
}

#[test]
fn row_number_accepts_timestamp_for_time_columns() {
    // v3 MDDS sends `ms_of_day` as a Timestamp.
    let epoch_ms: u64 = 1_775_050_200_000; // 2026-04-01 09:30 ET
    let row = row_of(vec![dv_timestamp(epoch_ms)]);
    assert_eq!(row_number(&row, 0).unwrap(), Some(34_200_000));
}

#[test]
fn row_text_errors_on_number_cell() {
    let row = row_of(vec![dv_number(42)]);
    assert_eq!(
        row_text(&row, 0),
        Err(DecodeError::TypeMismatch {
            column: 0,
            expected: "Text",
            observed: "Number",
        })
    );
}

#[test]
fn row_price_f64_accepts_number_cell() {
    // Documented v3 MDDS behavior: f64 fields may arrive as plain Number.
    let row = row_of(vec![dv_number(1_500_000)]);
    assert_eq!(row_price_f64(&row, 0).unwrap(), Some(1_500_000.0));
}

#[test]
fn row_price_value_returns_value_for_price_cell() {
    let row = row_of(vec![dv_price(12345, 10)]);
    assert_eq!(row_price_value(&row, 0).unwrap(), Some(12345));
}

#[test]
fn row_price_value_returns_none_for_null_cell() {
    let row = row_of(vec![dv_null()]);
    assert_eq!(row_price_value(&row, 0).unwrap(), None);
}

#[test]
fn row_price_type_returns_type_for_price_cell() {
    let row = row_of(vec![dv_price(12345, 10)]);
    assert_eq!(row_price_type(&row, 0).unwrap(), Some(10));
}

#[test]
fn row_price_type_returns_none_for_null_cell() {
    let row = row_of(vec![dv_null()]);
    assert_eq!(row_price_type(&row, 0).unwrap(), None);
}

#[test]
fn null_cells_dont_corrupt_trade_ticks() {
    // Build a minimal DataTable with one row that has a NullValue in a field.
    // Note: "price" header triggers Price-typed extraction, so we use a Price cell.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "sequence".into(),
            "ext_condition1".into(),
            "ext_condition2".into(),
            "ext_condition3".into(),
            "ext_condition4".into(),
            "condition".into(),
            "size".into(),
            "exchange".into(),
            "price".into(),
            "condition_flags".into(),
            "price_flags".into(),
            "volume_type".into(),
            "records_back".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34200000), // ms_of_day
            dv_number(1),        // sequence
            dv_null(),           // ext_condition1 = NullValue
            dv_number(0),        // ext_condition2
            dv_number(0),        // ext_condition3
            dv_number(0),        // ext_condition4
            dv_number(0),        // condition
            dv_number(100),      // size
            dv_number(4),        // exchange
            dv_price(15000, 10), // price (Price-typed because header is "price")
            dv_number(0),        // condition_flags
            dv_number(0),        // price_flags
            dv_number(0),        // volume_type
            dv_number(0),        // records_back
            dv_number(20240301), // date
        ])],
    };

    let ticks = parse_trade_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let tick = &ticks[0];
    assert_eq!(tick.ms_of_day, 34200000);
    // NullValue should default to 0, not corrupt subsequent fields.
    assert_eq!(tick.ext_condition1, 0);
    assert_eq!(tick.size, 100);
    assert!((tick.price - 15000.0).abs() < 1e-10);
    assert_eq!(tick.date, 20240301);
}

#[test]
fn extract_number_column_returns_none_for_null() {
    let table = proto::DataTable {
        headers: vec!["val".into()],
        data_table: vec![
            row_of(vec![dv_number(10)]),
            row_of(vec![dv_null()]),
            row_of(vec![dv_number(30)]),
        ],
    };

    let col = extract_number_column(&table, "val");
    assert_eq!(col, vec![Some(10), None, Some(30)]);
}

#[test]
fn parse_eod_timestamp_aliases_decode_time_and_date_separately() {
    // 2026-04-01 13:30:00 UTC = 2026-04-01 09:30:00 ET (EDT).
    let epoch_ms: u64 = 1_775_050_200_000;
    let table = proto::DataTable {
        headers: vec![
            "timestamp".into(),
            "timestamp2".into(),
            "open".into(),
            "close".into(),
        ],
        data_table: vec![row_of(vec![
            dv_timestamp(epoch_ms),
            dv_timestamp(epoch_ms),
            dv_number(15000),
            dv_number(15100),
        ])],
    };

    let ticks = parse_eod_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    assert_eq!(ticks[0].ms_of_day, 34_200_000);
    assert_eq!(ticks[0].ms_of_day2, 34_200_000);
    assert_eq!(ticks[0].date, 20260401);
    assert!((ticks[0].open - 15000.0).abs() < 1e-10);
    assert!((ticks[0].close - 15100.0).abs() < 1e-10);
}

#[test]
fn row_number_i64_decodes_price_cells() {
    // MDDS sends large integer fields as Price cells, not Number cells.
    // Price encoding: price_type centered at 10.
    //   type=10 → value as-is, type=13 → value * 10^3, type=7 → value / 10^3
    // Example: Price { value: 3842, type: 19 } = 3842 * 10^9 = 3_842_000_000_000
    let row = row_of(vec![dv_price(3842, 19)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap(),
        Some(3_842_000_000_000_i64)
    );
}

#[test]
fn row_number_i64_still_decodes_number_cells() {
    let row = row_of(vec![dv_number(999_999_999)]);
    assert_eq!(row_number_i64(&row, 0).unwrap(), Some(999_999_999));
}

#[test]
fn row_number_i64_returns_none_for_null() {
    let row = row_of(vec![dv_null()]);
    assert_eq!(row_number_i64(&row, 0).unwrap(), None);
}

#[test]
fn row_number_i64_errors_on_text_cell() {
    let row = row_of(vec![dv_text("oops")]);
    assert_eq!(
        row_number_i64(&row, 0),
        Err(DecodeError::TypeMismatch {
            column: 0,
            expected: "Number|Price",
            observed: "Text",
        })
    );
}

/// Pin a Price cell past `2^53` to the i64-native result for `type=17`.
#[test]
fn row_number_i64_price_cell_returns_bit_exact_i64() {
    let row = row_of(vec![dv_price(1_073_741_823, 17)]);
    let got = row_number_i64(&row, 0).unwrap().expect("Some");
    assert_eq!(got, 10_737_418_230_000_000_i64);
    assert!(got > (1_i64 << 53));
}

/// `value == 0` decodes to 0 regardless of the exponent. Mathematically
/// the product is zero; the decoder must not reject a zero cell, even
/// when `price_type` is at the clamp boundary.
#[test]
fn row_number_i64_price_zero_value_short_circuits() {
    let row = row_of(vec![dv_price(0, 19)]);
    assert_eq!(row_number_i64(&row, 0), Ok(Some(0)));
}

/// `row_number_i64` and `row_price_f64` must agree on the same wire
/// cell. With `type=19` (in-range) and `value=42`, `row_price_f64`
/// routes through `Price::new` which keeps `price_type=19`, and
/// `row_number_i64` produces the i64-native scale. Both should match.
/// Manual: 42 * 10^(19-10) = 42 * 10^9 = 42_000_000_000.
#[test]
fn row_number_i64_matches_row_price_f64_at_type_19() {
    let row = row_of(vec![dv_price(42, 19)]);
    let as_int = row_number_i64(&row, 0).unwrap().expect("Some");
    let as_float = row_price_f64(&row, 0).unwrap().expect("Some");
    assert_eq!(as_int, 42_000_000_000_i64);
    assert!((as_float - 42_000_000_000.0_f64).abs() < 1.0);
}

/// `price_type=20` is out-of-range; both decoders must clamp to 19
/// (matching `Price::new`). A `type=20` cell and a `type=19` cell with
/// the same value must therefore decode to the same i64.
#[test]
fn row_number_i64_clamps_price_type_above_19() {
    let row_clamped = row_of(vec![dv_price(7, 20)]);
    let row_in_range = row_of(vec![dv_price(7, 19)]);
    assert_eq!(
        row_number_i64(&row_clamped, 0).unwrap(),
        row_number_i64(&row_in_range, 0).unwrap(),
    );
    // Pin the absolute value too: 7 * 10^9 = 7_000_000_000.
    assert_eq!(
        row_number_i64(&row_clamped, 0).unwrap(),
        Some(7_000_000_000_i64)
    );
}

/// Maximum scale-up under the clamped contract: `value=i32::MAX,
/// type=19` yields `i32::MAX * 10^9 = 2_147_483_647_000_000_000`,
/// which is below `i64::MAX = 9_223_372_036_854_775_807`. The product
/// must fit and decode bit-exact (no `TypeMismatch`).
#[test]
fn row_number_i64_max_in_range_price_fits_i64() {
    let row = row_of(vec![dv_price(i32::MAX, 19)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap(),
        Some(2_147_483_647_000_000_000_i64),
    );
}

#[test]
fn parse_calendar_v3_holiday() {
    // Simulate calendar_year response for a holiday (full_close).
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into(), "open".into(), "close".into()],
        data_table: vec![row_of(vec![
            dv_text("2025-01-01"),
            dv_text("full_close"),
            dv_null(),
            dv_null(),
        ])],
    };

    let days = parse_calendar_days_v3(&table).unwrap();
    assert_eq!(days.len(), 1);
    let d = &days[0];
    assert_eq!(d.date, 20250101);
    assert_eq!(d.is_open, 0);
    assert_eq!(d.open_time, 0);
    assert_eq!(d.close_time, 0);
    assert_eq!(d.status, CALENDAR_STATUS_FULL_CLOSE);
}

#[test]
fn parse_calendar_v3_open_day() {
    // Simulate calendar_on_date response for a regular trading day.
    // Note: on_date and open_today omit the "date" column.
    let table = proto::DataTable {
        headers: vec!["type".into(), "open".into(), "close".into()],
        data_table: vec![row_of(vec![
            dv_text("open"),
            dv_text("09:30:00"),
            dv_text("16:00:00"),
        ])],
    };

    let days = parse_calendar_days_v3(&table).unwrap();
    assert_eq!(days.len(), 1);
    let d = &days[0];
    assert_eq!(d.date, 0); // no date column
    assert_eq!(d.is_open, 1);
    assert_eq!(d.open_time, 34_200_000); // 9:30 AM = 9*3600+30*60 = 34200 seconds = 34200000 ms
    assert_eq!(d.close_time, 57_600_000); // 4:00 PM = 16*3600 = 57600 seconds = 57600000 ms
    assert_eq!(d.status, CALENDAR_STATUS_OPEN);
}

#[test]
fn parse_calendar_v3_early_close() {
    // Simulate an early close day (day after Thanksgiving).
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into(), "open".into(), "close".into()],
        data_table: vec![row_of(vec![
            dv_text("2025-11-28"),
            dv_text("early_close"),
            dv_text("09:30:00"),
            dv_text("13:00:00"),
        ])],
    };

    let days = parse_calendar_days_v3(&table).unwrap();
    assert_eq!(days.len(), 1);
    let d = &days[0];
    assert_eq!(d.date, 20251128);
    assert_eq!(d.is_open, 1);
    assert_eq!(d.open_time, 34_200_000);
    assert_eq!(d.close_time, 46_800_000); // 1:00 PM = 13*3600 = 46800 seconds = 46800000 ms
    assert_eq!(d.status, CALENDAR_STATUS_EARLY_CLOSE);
}

#[test]
fn parse_calendar_v3_weekend() {
    let table = proto::DataTable {
        headers: vec!["type".into(), "open".into(), "close".into()],
        data_table: vec![row_of(vec![dv_text("weekend"), dv_null(), dv_null()])],
    };

    let days = parse_calendar_days_v3(&table).unwrap();
    assert_eq!(days.len(), 1);
    let d = &days[0];
    assert_eq!(d.is_open, 0);
    assert_eq!(d.status, CALENDAR_STATUS_WEEKEND);
}

#[test]
fn parse_time_text_valid() {
    assert_eq!(parse_time_text("09:30:00").unwrap(), 34_200_000);
    assert_eq!(parse_time_text("16:00:00").unwrap(), 57_600_000);
    assert_eq!(parse_time_text("13:00:00").unwrap(), 46_800_000);
    assert_eq!(parse_time_text("00:00:00").unwrap(), 0);
}

#[test]
fn parse_time_text_invalid_errors_with_raw_capture() {
    // SERIOUS #2 closure: malformed text time used to coalesce to 0,
    // silently corrupting downstream session timestamps. The strict
    // path surfaces the wire payload verbatim so operators can grep
    // for the failing value in upstream logs.
    assert_eq!(
        parse_time_text("invalid"),
        Err(DecodeError::InvalidTime {
            raw: "invalid".into()
        })
    );
    assert_eq!(
        parse_time_text(""),
        Err(DecodeError::InvalidTime { raw: "".into() })
    );
}

#[test]
fn parse_iso_date_yyyymmdd_passthrough_and_iso_split() {
    assert_eq!(parse_iso_date("20260413").unwrap(), 20260413);
    assert_eq!(parse_iso_date("2026-04-13").unwrap(), 20260413);
    // SERIOUS #2 closure: malformed text date used to coalesce to 0;
    // the strict path surfaces the raw payload as `InvalidDate` so
    // downstream timestamp consumers cannot silently mis-classify a
    // schema-drift case as the epoch.
    assert_eq!(
        parse_iso_date("not-a-date"),
        Err(DecodeError::InvalidDate {
            raw: "not-a-date".into()
        })
    );
}

#[test]
fn parse_trade_ticks_propagates_type_mismatch() {
    // A Text cell in an i32 column is a schema violation — the parser
    // must surface it, not silently coerce to 0.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into()],
        data_table: vec![row_of(vec![dv_text("not-a-number"), dv_price(15000, 10)])],
    };
    let err = parse_trade_ticks(&table).unwrap_err();
    assert!(
        matches!(err, DecodeError::TypeMismatch { .. }),
        "expected TypeMismatch, got {err:?}"
    );
}

// ─────────── Unset-oneof is an error at every strict decode site ───────────
//
// A `DataValue` with its `data_type` oneof unset is a wire-protocol
// anomaly (the upstream parser's default arm throws on it). The
// helpers `row_number` / `row_date` /
// etc. already surface it as `TypeMismatch { observed: "Unset" }`. These
// tests pin the same behaviour on the call-sites that used to coalesce
// `NullValue | None` to zero: `parse_option_contracts_v3`,
// `parse_calendar_days_v3`, the generator-emitted EOD helpers, and the
// generator-emitted contract-id injected `expiration` / `right` fields.

#[test]
fn parse_option_contracts_v3_errors_on_unset_expiration() {
    let table = proto::DataTable {
        headers: vec!["root".into(), "expiration".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_missing()])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::TypeMismatch {
            column: 1,
            expected: "Number|Text",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_option_contracts_v3_errors_on_unset_right() {
    let table = proto::DataTable {
        headers: vec!["root".into(), "right".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_missing()])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::TypeMismatch {
            column: 1,
            expected: "Number|Text",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_calendar_days_v3_errors_on_unset_date() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_missing(), dv_text("open")])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::TypeMismatch {
            column: 0,
            expected: "Number|Timestamp|Text",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_calendar_days_v3_errors_on_unset_open_time() {
    // `decode_calendar_time` is the helper covering both `open` and
    // `close`; one test pins the shared path.
    let table = proto::DataTable {
        headers: vec!["type".into(), "open".into(), "close".into()],
        data_table: vec![row_of(vec![
            dv_text("open"),
            dv_missing(),
            dv_text("16:00:00"),
        ])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::TypeMismatch {
            column: 1,
            expected: "Text|Number",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_eod_ticks_errors_on_unset_cell() {
    // `parse_eod_ticks` is generator-emitted with the `eod_num` /
    // `eod_date` / `eod_price` helpers; one test pins the shared path.
    let table = proto::DataTable {
        headers: vec!["timestamp".into(), "open".into()],
        data_table: vec![row_of(vec![dv_missing(), dv_number(15000)])],
    };
    let err = parse_eod_ticks(&table).unwrap_err();
    assert_eq!(
        err,
        DecodeError::TypeMismatch {
            column: 0,
            expected: "Number|Price|Timestamp",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_trade_ticks_errors_on_unset_injected_expiration() {
    // `parse_trade_ticks` is generator-emitted with `contract_id = true`;
    // an `expiration` header in the server payload triggers the injected
    // `expiration` / `strike` / `right` decode. An unset cell there used
    // to coalesce to 0; now it must fail loud.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into(), "expiration".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15000, 10),
            dv_missing(),
        ])],
    };
    let err = parse_trade_ticks(&table).unwrap_err();
    assert_eq!(
        err,
        DecodeError::TypeMismatch {
            column: 2,
            expected: "Number|Text",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_trade_ticks_errors_on_unset_injected_right() {
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into(), "right".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15000, 10),
            dv_missing(),
        ])],
    };
    let err = parse_trade_ticks(&table).unwrap_err();
    assert_eq!(
        err,
        DecodeError::TypeMismatch {
            column: 2,
            expected: "Number|Text",
            observed: "Unset",
        }
    );
}

#[test]
fn parse_greeks_all_ticks_decodes_price_encoded_greeks() {
    // Regression: an earlier strict decode rejected Price cells for Greek
    // columns, but the v3 MDDS server sends Greeks as Price-encoded
    // values (mirroring Java's `dataValue2Object` -> BigDecimal path).
    // Live run #24520486541 on main surfaced this as
    //   "column 13: expected Number, got Price"
    // on `option_snapshot_greeks_first_order::bulk_chain` and peers.
    // Pin Price-cell decoding for both IV and a Greek so a future
    // strict-Number tightening can't re-break it silently.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "implied_volatility".into(),
            "delta".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            // IV = 0.1234 encoded with price_type = 6 (value * 10^-4).
            dv_price(1234, 6),
            // Delta = 0.5 encoded with price_type = 9 (value * 10^-1).
            dv_price(5, 9),
        ])],
    };
    let ticks = parse_greeks_all_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    assert!((ticks[0].implied_volatility - 0.1234).abs() < 1e-10);
    assert!((ticks[0].delta - 0.5).abs() < 1e-10);
}

/// Pin the `implied_vol → implied_volatility` and `underlying_timestamp
/// → underlying_ms_of_day` aliases in `HEADER_ALIASES` by decoding a wire
/// payload whose headers use ONLY the v3 server-side names. If either
/// alias entry is dropped or mistyped, the matching schema field
/// silently zero-defaults via `opt_float` / `opt_number` (see the
/// generated `parse_greeks_all_ticks` body), and this test catches that
/// regression.
///
/// The companion fixture-driven test
/// `crates/thetadatadx/tests/test_decode_captures.rs::greeks_all_*`
/// can't catch a broken `implied_vol` alias on its own because the
/// captured fixture's `first_row_implied_volatility` is `0.0` — a
/// missing alias and a real zero IV are indistinguishable there.
#[test]
fn parse_greeks_all_ticks_resolves_implied_vol_and_underlying_timestamp_aliases() {
    // Headers use the v3 server-side names. Schema names
    // (`implied_volatility`, `underlying_ms_of_day`) are deliberately
    // absent so the parser MUST resolve them via `HEADER_ALIASES`.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "implied_vol".into(),
            "underlying_timestamp".into(),
        ],
        // IV = 0.42 encoded with price_type = 6 (value * 10^-4).
        // underlying_timestamp epoch_ms 1_775_050_200_000 corresponds
        // to 2026-04-01 09:30 ET, which `row_number` converts to
        // ms-of-day 34_200_000 (matching `first_row_underlying_ms_of_day`
        // in the option_history_greeks_all fixture meta).
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(4200, 6),
            dv_timestamp(1_775_050_200_000),
        ])],
    };
    let ticks = parse_greeks_all_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    // Non-zero IV proves the `implied_vol` alias resolved; a broken
    // alias would produce 0.0 from the `opt_float(None)` arm.
    assert!(
        (t.implied_volatility - 0.42).abs() < 1e-9,
        "implied_vol alias did not resolve: got {}",
        t.implied_volatility,
    );
    // Non-zero ms-of-day proves the `underlying_timestamp` alias
    // resolved; a broken alias would produce 0 from `opt_number(None)`.
    assert_eq!(t.underlying_ms_of_day, 34_200_000);
}

#[test]
fn parse_greeks_all_ticks_still_decodes_number_cells() {
    // Companion to the Price-cell regression test: Number cells must
    // still decode, matching Java's dispatch-on-wire-type semantics.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "implied_volatility".into()],
        data_table: vec![row_of(vec![dv_number(34_200_000), dv_number(0)])],
    };
    let ticks = parse_greeks_all_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    assert!(ticks[0].implied_volatility.abs() < 1e-10);
}

/// Vendor wire shape for `option_*_greeks_first_order`: only the seven
/// first-order columns plus IV pair — vanna/charm/vomma/veta/speed/
/// zomma/color/ultima/d1/d2/dual_delta/dual_gamma/vera are absent and
/// must default to `0.0` without surfacing any `find_header` warn.
/// Column layout pinned to `scripts/upstream_openapi.yaml` schema
/// `items_option_snapshot_greeks_first_order`.
#[test]
fn parse_greeks_all_ticks_decodes_first_order_subset_with_silent_gaps() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "implied_volatility".into(),
            "delta".into(),
            "theta".into(),
            "vega".into(),
            "rho".into(),
            "epsilon".into(),
            "lambda".into(),
            "iv_error".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(2142, 6),  // implied_volatility = 0.2142
            dv_price(5023, 6),  // delta = 0.5023
            dv_price(-114, 6),  // theta = -0.0114
            dv_price(8741, 6),  // vega = 0.8741
            dv_price(13598, 6), // rho = 1.3598
            dv_price(-1976, 6), // epsilon = -0.1976
            dv_price(32052, 6), // lambda = 3.2052
            dv_price(-3, 6),    // iv_error = -3 / 10^4 = -0.0003
            dv_number(20_240_614),
        ])],
    };
    let ticks = parse_greeks_all_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    // Wire-present columns: bit-exact against the input.
    // `dv_price(value, 6)` decodes as `value * 10^(6-10) = value / 10000`
    // (see `tdbe::types::price::Price::to_f64`).
    assert_eq!(t.ms_of_day, 34_200_000);
    assert!((t.implied_volatility - 0.2142).abs() < 1e-9);
    assert!((t.delta - 0.5023).abs() < 1e-9);
    assert!((t.theta - -0.0114).abs() < 1e-9);
    assert!((t.vega - 0.8741).abs() < 1e-9);
    assert!((t.rho - 1.3598).abs() < 1e-9);
    assert!((t.epsilon - -0.1976).abs() < 1e-9);
    assert!((t.lambda - 3.2052).abs() < 1e-9);
    assert!((t.iv_error - -0.0003).abs() < 1e-9);
    assert_eq!(t.date, 20_240_614);

    // Wire-absent columns: zero-defaulted. These are the columns the
    // server does NOT publish for `_greeks_first_order` — `find_header`
    // returning `None` for each must NOT yield an error and must NOT
    // warn (the pre-fix behaviour spammed eight warn lines per row).
    assert_eq!(t.gamma, 0.0);
    assert_eq!(t.vanna, 0.0);
    assert_eq!(t.charm, 0.0);
    assert_eq!(t.vomma, 0.0);
    assert_eq!(t.veta, 0.0);
    assert_eq!(t.speed, 0.0);
    assert_eq!(t.zomma, 0.0);
    assert_eq!(t.color, 0.0);
    assert_eq!(t.ultima, 0.0);
    assert_eq!(t.d1, 0.0);
    assert_eq!(t.d2, 0.0);
    assert_eq!(t.dual_delta, 0.0);
    assert_eq!(t.dual_gamma, 0.0);
    assert_eq!(t.vera, 0.0);
}

/// Vendor wire shape for `option_*_greeks_second_order`: gamma / vanna
/// / charm / vomma / veta plus IV pair. Column layout pinned to
/// upstream OpenAPI schema `items_option_snapshot_greeks_second_order`.
#[test]
fn parse_greeks_all_ticks_decodes_second_order_subset_with_silent_gaps() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "implied_volatility".into(),
            "gamma".into(),
            "vanna".into(),
            "charm".into(),
            "vomma".into(),
            "veta".into(),
            "iv_error".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(2142, 6), // implied_volatility = 0.2142
            dv_price(120, 6),  // gamma = 0.012
            dv_price(45, 6),   // vanna = 0.0045
            dv_price(-12, 6),  // charm = -0.0012
            dv_price(900, 6),  // vomma = 0.09
            dv_price(-3, 6),   // veta = -0.0003
            dv_price(-3, 6),   // iv_error = -0.0003
            dv_number(20_240_614),
        ])],
    };
    let ticks = parse_greeks_all_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    assert!((t.gamma - 0.012).abs() < 1e-9);
    assert!((t.vanna - 0.0045).abs() < 1e-9);
    assert!((t.charm - -0.0012).abs() < 1e-9);
    assert!((t.vomma - 0.09).abs() < 1e-9);
    assert!((t.veta - -0.0003).abs() < 1e-9);

    // First-order, third-order, and `_all`-only columns are absent
    // on the wire and default to 0.0.
    assert_eq!(t.delta, 0.0);
    assert_eq!(t.speed, 0.0);
    assert_eq!(t.zomma, 0.0);
    assert_eq!(t.d1, 0.0);
    assert_eq!(t.vera, 0.0);
}

/// Vendor wire shape for `option_*_greeks_third_order`: speed / zomma /
/// color / ultima plus IV pair. This is the exact endpoint the Issue
/// #472 reporter was hitting — `option_snapshot_greeks_third_order`
/// previously emitted eight warn lines per row for the absent
/// first-order / second-order / `_all`-only columns. The test pins the
/// silent-gap behaviour so a future regression of `find_header` back
/// to `tracing::warn!` would surface here as a behavioural change.
/// Column layout pinned to upstream OpenAPI schema
/// `items_option_snapshot_greeks_third_order` (notably `vera` is NOT
/// in the third-order subset; it only ships in `_greeks_all`).
#[test]
fn parse_greeks_all_ticks_decodes_third_order_subset_with_silent_gaps() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "implied_volatility".into(),
            "speed".into(),
            "zomma".into(),
            "color".into(),
            "ultima".into(),
            "iv_error".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(2142, 6), // implied_volatility = 0.2142
            dv_price(7, 6),    // speed  = 0.0007
            dv_price(15, 6),   // zomma  = 0.0015
            dv_price(-2, 6),   // color  = -0.0002
            dv_price(33, 6),   // ultima = 0.0033
            dv_price(-3, 6),   // iv_error = -0.0003
            dv_number(20_240_614),
        ])],
    };
    let ticks = parse_greeks_all_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    assert!((t.speed - 0.0007).abs() < 1e-9);
    assert!((t.zomma - 0.0015).abs() < 1e-9);
    assert!((t.color - -0.0002).abs() < 1e-9);
    assert!((t.ultima - 0.0033).abs() < 1e-9);

    // Vera is NOT a third-order column on the wire even though the
    // generic `GreeksTick` struct carries the field. It must default
    // to 0.0 here without warning.
    assert_eq!(t.vera, 0.0);
    // First-order and second-order columns also absent.
    assert_eq!(t.delta, 0.0);
    assert_eq!(t.gamma, 0.0);
    assert_eq!(t.vanna, 0.0);
    assert_eq!(t.d1, 0.0);
    assert_eq!(t.dual_gamma, 0.0);
}

/// `parse_greeks_first_order_ticks` against the column subset the
/// vendor publishes for `option_*_greeks_first_order` -- pinned to
/// `items_option_snapshot_greeks_first_order` in the upstream OpenAPI.
/// Asserts every column the parser fills decodes to the exact value
/// from the input row, and that the underlying-snapshot pair is
/// populated (the column subset is what differs from `_greeks_all`,
/// not the underlying tail).
#[test]
fn parse_greeks_first_order_ticks_decodes_first_order_subset() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid".into(),
            "ask".into(),
            "delta".into(),
            "theta".into(),
            "vega".into(),
            "rho".into(),
            "epsilon".into(),
            "lambda".into(),
            "implied_volatility".into(),
            "iv_error".into(),
            "underlying_ms_of_day".into(),
            "underlying_price".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15022, 6), // bid = 1.5022
            dv_price(15041, 6), // ask = 1.5041
            dv_price(5023, 6),  // delta = 0.5023
            dv_price(-114, 6),  // theta = -0.0114
            dv_price(8741, 6),  // vega = 0.8741
            dv_price(13598, 6), // rho = 1.3598
            dv_price(-1976, 6), // epsilon = -0.1976
            dv_price(32052, 6), // lambda = 3.2052
            dv_price(2142, 6),  // implied_volatility = 0.2142
            dv_price(-3, 6),    // iv_error = -0.0003
            dv_number(34_200_001),
            dv_price(580025, 6), // underlying_price = 58.0025
            dv_number(20_240_614),
        ])],
    };
    let ticks = parse_greeks_first_order_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    assert_eq!(t.ms_of_day, 34_200_000);
    assert!((t.bid - 1.5022).abs() < 1e-9);
    assert!((t.ask - 1.5041).abs() < 1e-9);
    assert!((t.delta - 0.5023).abs() < 1e-9);
    assert!((t.theta - -0.0114).abs() < 1e-9);
    assert!((t.vega - 0.8741).abs() < 1e-9);
    assert!((t.rho - 1.3598).abs() < 1e-9);
    assert!((t.epsilon - -0.1976).abs() < 1e-9);
    assert!((t.lambda - 3.2052).abs() < 1e-9);
    assert!((t.implied_volatility - 0.2142).abs() < 1e-9);
    assert!((t.iv_error - -0.0003).abs() < 1e-9);
    assert_eq!(t.underlying_ms_of_day, 34_200_001);
    assert!((t.underlying_price - 58.0025).abs() < 1e-9);
    assert_eq!(t.date, 20_240_614);
}

/// `parse_greeks_second_order_ticks` against the column subset the
/// vendor publishes for `option_*_greeks_second_order` -- pinned to
/// `items_option_snapshot_greeks_second_order` in the upstream
/// OpenAPI. Second-order Greeks: gamma / vanna / charm / vomma /
/// veta plus the IV pair and the bid/ask quote pair.
#[test]
fn parse_greeks_second_order_ticks_decodes_second_order_subset() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid".into(),
            "ask".into(),
            "gamma".into(),
            "vanna".into(),
            "charm".into(),
            "vomma".into(),
            "veta".into(),
            "implied_volatility".into(),
            "iv_error".into(),
            "underlying_ms_of_day".into(),
            "underlying_price".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15022, 6), // bid = 1.5022
            dv_price(15041, 6), // ask = 1.5041
            dv_price(120, 6),   // gamma = 0.012
            dv_price(45, 6),    // vanna = 0.0045
            dv_price(-12, 6),   // charm = -0.0012
            dv_price(900, 6),   // vomma = 0.09
            dv_price(-3, 6),    // veta = -0.0003
            dv_price(2142, 6),  // implied_volatility = 0.2142
            dv_price(-3, 6),    // iv_error = -0.0003
            dv_number(34_200_001),
            dv_price(580025, 6),
            dv_number(20_240_614),
        ])],
    };
    let ticks = parse_greeks_second_order_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    assert_eq!(t.ms_of_day, 34_200_000);
    assert!((t.bid - 1.5022).abs() < 1e-9);
    assert!((t.ask - 1.5041).abs() < 1e-9);
    assert!((t.gamma - 0.012).abs() < 1e-9);
    assert!((t.vanna - 0.0045).abs() < 1e-9);
    assert!((t.charm - -0.0012).abs() < 1e-9);
    assert!((t.vomma - 0.09).abs() < 1e-9);
    assert!((t.veta - -0.0003).abs() < 1e-9);
    assert!((t.implied_volatility - 0.2142).abs() < 1e-9);
    assert!((t.iv_error - -0.0003).abs() < 1e-9);
    assert_eq!(t.underlying_ms_of_day, 34_200_001);
    assert!((t.underlying_price - 58.0025).abs() < 1e-9);
    assert_eq!(t.date, 20_240_614);
}

/// `parse_greeks_third_order_ticks` against the column subset the
/// vendor publishes for `option_*_greeks_third_order` -- pinned to
/// `items_option_snapshot_greeks_third_order` in the upstream
/// OpenAPI. Third-order Greeks: speed / zomma / color / ultima plus
/// the IV pair and the bid/ask quote pair. Notably the wire schema
/// does NOT publish `vera`; the struct does not carry it either.
#[test]
fn parse_greeks_third_order_ticks_decodes_third_order_subset() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid".into(),
            "ask".into(),
            "speed".into(),
            "zomma".into(),
            "color".into(),
            "ultima".into(),
            "implied_volatility".into(),
            "iv_error".into(),
            "underlying_ms_of_day".into(),
            "underlying_price".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15022, 6), // bid = 1.5022
            dv_price(15041, 6), // ask = 1.5041
            dv_price(7, 6),     // speed = 0.0007
            dv_price(15, 6),    // zomma = 0.0015
            dv_price(-2, 6),    // color = -0.0002
            dv_price(33, 6),    // ultima = 0.0033
            dv_price(2142, 6),  // implied_volatility = 0.2142
            dv_price(-3, 6),    // iv_error = -0.0003
            dv_number(34_200_001),
            dv_price(580025, 6),
            dv_number(20_240_614),
        ])],
    };
    let ticks = parse_greeks_third_order_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    assert_eq!(t.ms_of_day, 34_200_000);
    assert!((t.bid - 1.5022).abs() < 1e-9);
    assert!((t.ask - 1.5041).abs() < 1e-9);
    assert!((t.speed - 0.0007).abs() < 1e-9);
    assert!((t.zomma - 0.0015).abs() < 1e-9);
    assert!((t.color - -0.0002).abs() < 1e-9);
    assert!((t.ultima - 0.0033).abs() < 1e-9);
    assert!((t.implied_volatility - 0.2142).abs() < 1e-9);
    assert!((t.iv_error - -0.0003).abs() < 1e-9);
    assert_eq!(t.underlying_ms_of_day, 34_200_001);
    assert!((t.underlying_price - 58.0025).abs() < 1e-9);
    assert_eq!(t.date, 20_240_614);
}

// ─────────── Subset NBBO header set: decoder must zero-fill absent
// ─────────── exchange/condition columns
//
// Defense-in-depth: a `DataTable` whose header set is a subset of
// the canonical NBBO schema (six of eleven columns present, with
// `bid_exchange`, `bid_condition`, `ask_exchange`, `ask_condition`
// absent) must decode without error and zero-fill the absent
// columns. The subset layout
// `[ms_of_day, bid_size, bid, ask_size, ask, date]` is the
// canonical shape these tests exercise.
//
// This pair of tests pins that behaviour so a future regression in
// `find_header` / the generator's `opt_number(idx)` arm surfaces
// here.

/// A `DataTable` whose headers match the subset NBBO shape
/// (`ms_of_day, bid_size, bid, ask_size, ask, date`) must decode to a
/// `QuoteTick` with the absent exchange / condition columns zero-filled.
#[test]
fn quote_tick_decodes_legacy_six_field_shape_with_zero_fill() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid_size".into(),
            "bid".into(),
            "ask_size".into(),
            "ask".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_number(50),
            dv_price(15022, 6), // bid = 1.5022
            dv_number(75),
            dv_price(15041, 6), // ask = 1.5041
            dv_number(20_220_414),
        ])],
    };
    let ticks = parse_quote_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    // Wire-present columns decode bit-exact.
    assert_eq!(t.ms_of_day, 34_200_000);
    assert_eq!(t.bid_size, 50);
    assert!((t.bid - 1.5022).abs() < 1e-9);
    assert_eq!(t.ask_size, 75);
    assert!((t.ask - 1.5041).abs() < 1e-9);
    assert_eq!(t.date, 20_220_414);

    // Wire-absent columns zero-fill: mirrors the gRPC decoder's
    // `opt_number(row, None) -> 0` contract.
    assert_eq!(t.bid_exchange, 0);
    assert_eq!(t.bid_condition, 0);
    assert_eq!(t.ask_exchange, 0);
    assert_eq!(t.ask_condition, 0);

    // Midpoint is computed from bid + ask regardless of legacy / current
    // layout — pin the value so a generator regression on the midpoint
    // post-processing step would surface here.
    assert!((t.midpoint - 1.50315).abs() < 1e-9);
}

/// The full 11-field shape must continue to decode all columns. A
/// fix that accidentally narrowed the parser to the 6-field subset
/// layout would surface as wrong values on `bid_exchange` / `ask_condition`.
#[test]
fn quote_tick_decodes_current_eleven_field_shape_unchanged() {
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid_size".into(),
            "bid_exchange".into(),
            "bid".into(),
            "bid_condition".into(),
            "ask_size".into(),
            "ask_exchange".into(),
            "ask".into(),
            "ask_condition".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_number(50),
            dv_number(7), // CBOE
            dv_price(15022, 6),
            dv_number(1),
            dv_number(75),
            dv_number(8), // NYSE Arca
            dv_price(15041, 6),
            dv_number(2),
            dv_number(20_240_605),
        ])],
    };
    let ticks = parse_quote_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];

    assert_eq!(t.ms_of_day, 34_200_000);
    assert_eq!(t.bid_size, 50);
    assert_eq!(t.bid_exchange, 7);
    assert!((t.bid - 1.5022).abs() < 1e-9);
    assert_eq!(t.bid_condition, 1);
    assert_eq!(t.ask_size, 75);
    assert_eq!(t.ask_exchange, 8);
    assert!((t.ask - 1.5041).abs() < 1e-9);
    assert_eq!(t.ask_condition, 2);
    assert_eq!(t.date, 20_240_605);
}

// ─────────────────── SERIOUS #2: invalid-text propagation ───────────────────
//
// The v3 wire path used to coalesce malformed date / time text to `0`.
// That silently corrupted downstream timestamps when the upstream
// schema drifted; the strict path now surfaces it as
// `DecodeError::InvalidDate { raw }` / `DecodeError::InvalidTime { raw }`
// so operators can grep for the failing payload in their logs.

#[test]
fn parse_calendar_days_v3_errors_on_invalid_date_text() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_text("not-a-date"), dv_text("open")])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "not-a-date".into(),
        }
    );
}

// ─────────────────── SERIOUS #3: unknown-enum-text propagation ───────────────────
//
// The v3 wire path used to fall through to `0` (right) or
// `CALENDAR_STATUS_UNKNOWN` (calendar type) on text values outside the
// documented vocabulary. That silently masked upstream schema drift;
// the strict path now surfaces it as
// `DecodeError::UnknownEnumVariant { field, raw }` so operators can
// grep for the unrecognised payload in their logs.

#[test]
fn parse_option_contracts_v3_errors_on_unknown_right_text() {
    let table = proto::DataTable {
        headers: vec!["root".into(), "right".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_text("Q")])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "right",
            raw: "Q".into(),
        }
    );
}

#[test]
fn parse_calendar_days_v3_errors_on_unknown_type_text() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_number(20_260_413), dv_text("partial")])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "calendar.type",
            raw: "partial".into(),
        }
    );
}

#[test]
fn parse_calendar_days_v3_errors_on_invalid_open_time_text() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into(), "open".into()],
        data_table: vec![row_of(vec![
            dv_number(20_260_413),
            dv_text("open"),
            dv_text("invalid"),
        ])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::InvalidTime {
            raw: "invalid".into(),
        }
    );
}

#[test]
fn parse_option_contracts_v3_errors_on_invalid_expiration_text() {
    let table = proto::DataTable {
        headers: vec!["root".into(), "expiration".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_text("not-a-date")])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "not-a-date".into(),
        }
    );
}
