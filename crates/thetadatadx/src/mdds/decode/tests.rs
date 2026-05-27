//! Tests for the row-cell decoders, column extractors, and v3 hand-written
//! parsers. Eastern-time / DST primitive tests live with their canonical
//! home in `tdbe::time`.

use super::cell::{
    row_date, row_number, row_number_i64, row_price_f64, row_price_type, row_price_value, row_text,
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

/// `price_type=20` is out-of-range; both decoders must surface a
/// typed `InvalidPriceType` error rather than silently saturating to
/// `19` (which previously produced wrong-magnitude downstream
/// prices). Boundary check at `MAX_PRICE_TYPE + 1`.
#[test]
fn row_number_i64_rejects_price_type_above_max() {
    let row = row_of(vec![dv_price(7, 20)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap_err(),
        DecodeError::InvalidPriceType { raw: 20 },
    );
}

/// Companion check for `row_price_f64` so the two decoders share the
/// same wire-protocol contract on out-of-range `price_type`. Same
/// boundary cell as `row_number_i64_rejects_price_type_above_max`.
#[test]
fn row_price_f64_rejects_price_type_above_max() {
    let row = row_of(vec![dv_price(7, 20)]);
    assert_eq!(
        row_price_f64(&row, 0).unwrap_err(),
        DecodeError::InvalidPriceType { raw: 20 },
    );
}

/// `price_type=21` — one past the boundary, still rejected.
#[test]
fn row_number_i64_rejects_price_type_21() {
    let row = row_of(vec![dv_price(7, 21)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap_err(),
        DecodeError::InvalidPriceType { raw: 21 },
    );
}

/// `price_type=100` — well outside the documented vendor range.
#[test]
fn row_number_i64_rejects_price_type_100() {
    let row = row_of(vec![dv_price(7, 100)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap_err(),
        DecodeError::InvalidPriceType { raw: 100 },
    );
}

/// `price_type=i32::MAX` — pathological upper extreme; the wire-
/// protocol error must still surface verbatim.
#[test]
fn row_number_i64_rejects_price_type_i32_max() {
    let row = row_of(vec![dv_price(7, i32::MAX)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap_err(),
        DecodeError::InvalidPriceType { raw: i32::MAX },
    );
}

/// `price_type=-1` — negative wire payload also rejected (matches
/// `Price::with_value_and_type`'s `0..=19` contract).
#[test]
fn row_number_i64_rejects_negative_price_type() {
    let row = row_of(vec![dv_price(7, -1)]);
    assert_eq!(
        row_number_i64(&row, 0).unwrap_err(),
        DecodeError::InvalidPriceType { raw: -1 },
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

// ─────── Wave-6: calendar-range rejection on parse_iso_date / parse_time_text ───────
//
// Wave-5 closed the "coalesce to 0 on parse failure" hole but left
// the calendar-range hole open: shape-valid impossibilities like
// `20260230` (Feb 30) or `2026-13-01` (month 13) still slipped
// through because the parser only checked that the digits split into
// the right number of integer components. Wave-6 routes both shapes
// through `tdbe::time::is_valid_gregorian_date` so the strict-decode
// contract matches the v3 vendor reality: the wire only ever
// publishes real Gregorian dates, and anything else is upstream
// drift the operator needs to see.

#[test]
fn parse_iso_date_rejects_compact_feb_30() {
    // Feb 30 never exists in any year — the most flagged shape in
    // the codex revalidation.
    assert_eq!(
        parse_iso_date("20260230"),
        Err(DecodeError::InvalidDate {
            raw: "20260230".into(),
        }),
    );
}

#[test]
fn parse_iso_date_rejects_iso_month_13() {
    // Month component exceeds 12 — out-of-range under the canonical
    // Gregorian validator.
    assert_eq!(
        parse_iso_date("2026-13-01"),
        Err(DecodeError::InvalidDate {
            raw: "2026-13-01".into(),
        }),
    );
}

#[test]
fn parse_iso_date_rejects_iso_feb_29_non_leap() {
    // 2026 % 4 != 0 — not a leap year, so Feb 29 is invalid.
    assert_eq!(
        parse_iso_date("2026-02-29"),
        Err(DecodeError::InvalidDate {
            raw: "2026-02-29".into(),
        }),
    );
}

#[test]
fn parse_iso_date_accepts_iso_feb_29_real_leap() {
    // 2024 is a leap year — Feb 29 is real and must round-trip
    // through the validator.
    assert_eq!(parse_iso_date("2024-02-29").unwrap(), 20240229);
}

#[test]
fn parse_iso_date_rejects_compact_year_zero() {
    // The `00000000` sentinel must not flow through to downstream
    // timestamp arithmetic.
    assert_eq!(
        parse_iso_date("00000000"),
        Err(DecodeError::InvalidDate {
            raw: "00000000".into(),
        }),
    );
}

#[test]
fn parse_time_text_rejects_hour_25() {
    // Hour component exceeds the 0..=23 clock range.
    assert_eq!(
        parse_time_text("25:00:00"),
        Err(DecodeError::InvalidTime {
            raw: "25:00:00".into(),
        }),
    );
}

#[test]
fn parse_time_text_rejects_minute_61() {
    // Minute component exceeds the 0..=59 clock range.
    assert_eq!(
        parse_time_text("12:61:00"),
        Err(DecodeError::InvalidTime {
            raw: "12:61:00".into(),
        }),
    );
}

#[test]
fn parse_time_text_rejects_second_61() {
    // Second component exceeds the 0..=59 clock range (no leap
    // seconds on the wire).
    assert_eq!(
        parse_time_text("12:00:61"),
        Err(DecodeError::InvalidTime {
            raw: "12:00:61".into(),
        }),
    );
}

#[test]
fn parse_time_text_rejects_all_three_out_of_range() {
    // Pathological "25:61:61" — every component outside its range.
    assert_eq!(
        parse_time_text("25:61:61"),
        Err(DecodeError::InvalidTime {
            raw: "25:61:61".into(),
        }),
    );
}

#[test]
fn parse_time_text_rejects_negative_hour() {
    // Negative hour is a wire-protocol anomaly the strict path
    // must surface verbatim.
    assert_eq!(
        parse_time_text("-1:00:00"),
        Err(DecodeError::InvalidTime {
            raw: "-1:00:00".into(),
        }),
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

// ───── Wave-7: numeric YYYYMMDD wire arms route through is_valid_yyyymmdd ─────
//
// Round-1 hardening of `parse_iso_date` only covered the Text arm, but
// real v3 MDDS payloads carry packed YYYYMMDD dates as `Number(n)` on
// `row_date`, `parse_option_contracts_v3::expiration`, and
// `parse_calendar_days_v3::date`. Pre-Wave-7 those numeric arms cast
// straight to i32 with no calendar check, so `Number(20260230)` (Feb 30)
// and `Number(20261301)` (month 13) decoded silently. Each numeric arm
// now routes through `tdbe::time::is_valid_yyyymmdd` and surfaces
// `DecodeError::InvalidDate` with the raw integer attached.

#[test]
fn row_date_rejects_number_feb_30() {
    let row = row_of(vec![dv_number(20_260_230)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate {
            raw: "20260230".into(),
        })
    );
}

#[test]
fn row_date_rejects_number_month_13() {
    let row = row_of(vec![dv_number(20_261_301)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate {
            raw: "20261301".into(),
        })
    );
}

#[test]
fn row_date_accepts_number_real_leap_day() {
    // 2024 is a leap year — Feb 29 is real and must round-trip through
    // the validator unchanged.
    let row = row_of(vec![dv_number(20_240_229)]);
    assert_eq!(row_date(&row, 0).unwrap(), Some(20_240_229));
}

#[test]
fn row_date_rejects_number_feb_29_non_leap() {
    // 2025 % 4 != 0 — Feb 29 is calendar-impossible.
    let row = row_of(vec![dv_number(20_250_229)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate {
            raw: "20250229".into(),
        })
    );
}

#[test]
fn row_date_rejects_number_zero() {
    // The `00000000` sentinel must not flow through to downstream
    // timestamp arithmetic — `is_valid_yyyymmdd(0)` is false.
    let row = row_of(vec![dv_number(0)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate { raw: "0".into() })
    );
}

#[test]
fn row_date_accepts_real_date_unchanged() {
    // Sanity: a real Gregorian date round-trips with no error.
    let row = row_of(vec![dv_number(20_260_413)]);
    assert_eq!(row_date(&row, 0).unwrap(), Some(20_260_413));
}

#[test]
fn parse_option_contracts_v3_rejects_numeric_expiration_feb_30() {
    let table = proto::DataTable {
        headers: vec!["root".into(), "expiration".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(20_260_230)])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20260230".into(),
        }
    );
}

#[test]
fn parse_option_contracts_v3_rejects_numeric_expiration_month_13() {
    let table = proto::DataTable {
        headers: vec!["root".into(), "expiration".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(20_261_301)])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20261301".into(),
        }
    );
}

#[test]
fn parse_option_contracts_v3_accepts_numeric_expiration_real_date() {
    // Sanity check the numeric arm still produces a valid contract for
    // a real Gregorian expiration.
    let table = proto::DataTable {
        headers: vec!["root".into(), "expiration".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(20_240_229)])],
    };
    let contracts = parse_option_contracts_v3(&table).unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].expiration, 20_240_229);
}

#[test]
fn parse_calendar_days_v3_rejects_numeric_date_feb_30() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_number(20_260_230), dv_text("open")])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20260230".into(),
        }
    );
}

#[test]
fn parse_calendar_days_v3_rejects_numeric_date_month_13() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_number(20_261_301), dv_text("open")])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20261301".into(),
        }
    );
}

#[test]
fn parse_calendar_days_v3_accepts_numeric_date_real_leap_day() {
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_number(20_240_229), dv_text("open")])],
    };
    let days = parse_calendar_days_v3(&table).unwrap();
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].date, 20_240_229);
}

// ─────────── Generator-emitted contract_id expiration arm ───────────
//
// Round-2 hardened `eod_date`, `row_date`, and `parse_iso_date` so the
// hand-written numeric date paths reject calendar-impossible payloads.
// The generator template that inlines `expiration` into every parser
// with `contract_id = true` was missed — `Number(n) -> *n as i32` still
// cast straight through. That affected 18 public parsers including
// `parse_trade_ticks`, `parse_quote_ticks`, `parse_eod_ticks`, and
// every greeks variant. These tests pin the canonical `InvalidDate`
// behaviour across a representative sample of the affected surface
// (one of each: i32-style, quote-style, eod-style, greeks-style) on
// Feb-30, month-13, non-leap Feb-29, and the valid leap-day shapes.

#[test]
fn parse_trade_ticks_rejects_numeric_expiration_feb_30() {
    // `parse_trade_ticks` injects the contract_id arm when an
    // `expiration` header is present in the server payload. Number
    // arms used to cast straight to i32 with no Gregorian check;
    // `Number(20260230)` (Feb 30) must now raise the canonical
    // `InvalidDate { raw: "20260230" }` instead of propagating
    // through to downstream timestamp arithmetic.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into(), "expiration".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_number(20_260_230),
        ])],
    };
    assert_eq!(
        parse_trade_ticks(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20260230".into(),
        }
    );
}

#[test]
fn parse_quote_ticks_rejects_numeric_expiration_month_13() {
    // Quote surface — same generator template, different parser. The
    // month-13 payload tests the high-half of the YYYYMMDD validator.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid".into(),
            "ask".into(),
            "expiration".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_price(15_100, 10),
            dv_number(20_261_301),
        ])],
    };
    assert_eq!(
        parse_quote_ticks(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20261301".into(),
        }
    );
}

#[test]
fn parse_eod_ticks_accepts_numeric_expiration_real_leap_day() {
    // 2024 is a leap year — Feb 29 is a real Gregorian date and must
    // round-trip unchanged. Sanity check that the new validator does
    // not over-reject legitimate expirations on the eod surface.
    let table = proto::DataTable {
        headers: vec!["timestamp".into(), "open".into(), "expiration".into()],
        data_table: vec![row_of(vec![
            dv_timestamp(1_775_050_200_000),
            dv_number(15_000),
            dv_number(20_240_229),
        ])],
    };
    let ticks = parse_eod_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    assert_eq!(ticks[0].expiration, 20_240_229);
}

#[test]
fn parse_greeks_all_ticks_rejects_numeric_expiration_non_leap_feb_29() {
    // 2025 % 4 != 0 — Feb 29 is calendar-impossible. The non-leap
    // boundary is the failure mode most likely to slip through a
    // naive "month/day in range" check; the Gregorian validator
    // catches it. Greeks surface confirms the same template applies
    // across every contract_id tick type.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid".into(),
            "ask".into(),
            "delta".into(),
            "expiration".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_price(15_100, 10),
            dv_price(500, 13),
            dv_number(20_250_229),
        ])],
    };
    assert_eq!(
        parse_greeks_all_ticks(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "20250229".into(),
        }
    );
}

// ─────────── Generator-emitted contract_id right arm ───────────
//
// Sibling-arm cleanup: the hand-written `parse_option_contracts_v3`
// right-text arm surfaces unknown text as `UnknownEnumVariant`, but
// the generator template silently coalesced unknown right strings to
// `0`. A future server change (e.g. introducing a new option style)
// would have masked schema drift. Mirror the canonical strict-decode
// policy on the generator surface too.

#[test]
fn parse_trade_ticks_rejects_unknown_right_text() {
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into(), "right".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_text("STRADDLE"),
        ])],
    };
    assert_eq!(
        parse_trade_ticks(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "right",
            raw: "STRADDLE".into(),
        }
    );
}

// ─────────── Numeric date arms: int64 overflow guards ───────────
//
// Round-3 wrapped the numeric date arms in `is_valid_yyyymmdd`, but the
// validator ran on `*n as i32` and `DataValue.number` is wire-typed
// `int64`. A drifted or hostile payload like `4_315_207_525` (==
// `(1 << 32) + 20_240_229`) truncates cleanly to `20_240_229` and slips
// past the Gregorian check. Every numeric date arm — `row_date`,
// `parse_option_contracts_v3` expiration, `parse_calendar_days_v3` date,
// the generator-emitted contract_id expiration template, the eod_date
// template — now goes through `i32::try_from` first and raises
// `DecodeError::InvalidDate` with the raw int64 captured verbatim.

#[test]
fn row_date_rejects_number_overflowing_i32_low_bits_look_valid() {
    // 4_315_207_525 == (1 << 32) + 20_240_229. As i64 it's well outside
    // i32 range; the low 32 bits decode to a real leap-day date that
    // would otherwise pass the Gregorian check.
    let row = row_of(vec![dv_number(4_315_207_525)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate {
            raw: "4315207525".into(),
        })
    );
}

#[test]
fn row_date_rejects_number_i64_max() {
    let row = row_of(vec![dv_number(i64::MAX)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate {
            raw: i64::MAX.to_string(),
        })
    );
}

#[test]
fn row_date_rejects_negative_one() {
    // -1 fits in i32 (`as i32` keeps -1) but the Gregorian validator
    // rejects negative years. The raw int captured for diagnostics is
    // the original int64.
    let row = row_of(vec![dv_number(-1)]);
    assert_eq!(
        row_date(&row, 0),
        Err(DecodeError::InvalidDate { raw: "-1".into() })
    );
}

#[test]
fn parse_option_contracts_v3_rejects_numeric_expiration_overflowing_i32() {
    // i32::MAX + 1 — outside the documented YYYYMMDD width. Without the
    // try_from guard the wrap would produce i32::MIN and short-circuit
    // through `is_valid_yyyymmdd` as a negative year reject; with the
    // guard the raw int64 is captured verbatim instead.
    let table = proto::DataTable {
        headers: vec!["root".into(), "expiration".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(2_147_483_648)])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "2147483648".into(),
        }
    );
}

#[test]
fn parse_trade_ticks_rejects_numeric_expiration_overflowing_i32() {
    // Generator-emitted contract_id expiration arm. 4_315_207_525 ==
    // (1 << 32) + 20_240_229 — the low 32 bits look like a real leap
    // day, which is exactly the failure mode the validator could not
    // catch when fed `*n as i32`.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into(), "expiration".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_number(4_315_207_525),
        ])],
    };
    assert_eq!(
        parse_trade_ticks(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "4315207525".into(),
        }
    );
}

#[test]
fn parse_eod_ticks_rejects_numeric_date_overflowing_i32() {
    // The eod_date helper template is the second canonical date path
    // emitted by the generator. Drive an overflow through the EOD
    // surface via the `date` column so the eod_date Number arm runs.
    let table = proto::DataTable {
        headers: vec!["date".into(), "open".into()],
        data_table: vec![row_of(vec![dv_number(i64::MAX), dv_number(15_000)])],
    };
    assert_eq!(
        parse_eod_ticks(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: i64::MAX.to_string(),
        }
    );
}

#[test]
fn parse_calendar_days_v3_rejects_numeric_date_overflowing_i32() {
    // `parse_calendar_days_v3` shares the same numeric-date pattern as
    // `parse_option_contracts_v3` and was a sibling-arm miss in the
    // round-3 sweep. The int64 overflow surfaces with the raw value
    // captured verbatim.
    let table = proto::DataTable {
        headers: vec!["date".into(), "type".into()],
        data_table: vec![row_of(vec![dv_number(4_315_207_525), dv_text("open")])],
    };
    assert_eq!(
        parse_calendar_days_v3(&table).unwrap_err(),
        DecodeError::InvalidDate {
            raw: "4315207525".into(),
        }
    );
}

// ─────────── Numeric right arm: canonical CALL/PUT byte guard ───────────
//
// Round-3 fixed only the text arm of the `right` field. The numeric arm
// in both the contract_right generator template and
// `parse_option_contracts_v3` still cast `Number(n) as i32` and stored
// arbitrary values — `Number(81)`, `Number(0)`, and any overflowing
// int64 silently became contract rights across every wildcard parser
// with `contract_id = true` plus the hand-written option-contracts
// surface. Both numeric arms now accept only the canonical ASCII bytes
// 67 (`'C'`) and 80 (`'P'`); anything else (including int64 overflow)
// raises `UnknownEnumVariant` with the raw value captured verbatim.

#[test]
fn parse_option_contracts_v3_rejects_numeric_right_81() {
    // 81 is one off from `'P'` (80) — a plausible drift on an upstream
    // server that reshuffles its right enum, and exactly the failure
    // mode the silent cast was masking.
    let table = proto::DataTable {
        headers: vec!["root".into(), "right".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(81)])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "right",
            raw: "81".into(),
        }
    );
}

#[test]
fn parse_option_contracts_v3_rejects_numeric_right_zero() {
    // 0 was the silent-coalesce sentinel before the strict-decode
    // policy landed; verify it now raises loud.
    let table = proto::DataTable {
        headers: vec!["root".into(), "right".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(0)])],
    };
    assert_eq!(
        parse_option_contracts_v3(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "right",
            raw: "0".into(),
        }
    );
}

#[test]
fn parse_option_contracts_v3_accepts_numeric_right_call_byte() {
    // 67 == ASCII 'C' — the canonical CALL wire byte. Must round-trip
    // unchanged so the new bounds check does not over-reject the
    // documented payload shape.
    let table = proto::DataTable {
        headers: vec!["root".into(), "right".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(67)])],
    };
    let contracts = parse_option_contracts_v3(&table).unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].right, 67);
}

#[test]
fn parse_option_contracts_v3_accepts_numeric_right_put_byte() {
    // 80 == ASCII 'P' — the canonical PUT wire byte.
    let table = proto::DataTable {
        headers: vec!["root".into(), "right".into()],
        data_table: vec![row_of(vec![dv_text("AAPL"), dv_number(80)])],
    };
    let contracts = parse_option_contracts_v3(&table).unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].right, 80);
}

#[test]
fn parse_trade_ticks_rejects_numeric_right_81() {
    // Generator-emitted contract_id right arm. Same sibling-arm miss as
    // `parse_option_contracts_v3` — pin the canonical strict-decode
    // policy on the generator surface too.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into(), "right".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_number(81),
        ])],
    };
    assert_eq!(
        parse_trade_ticks(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "right",
            raw: "81".into(),
        }
    );
}

#[test]
fn parse_quote_ticks_rejects_numeric_right_overflowing_i32() {
    // int64 overflow on the generator-emitted right arm — without the
    // try_from guard the wrap would silently produce some i32 value
    // and store it. The raw int64 must be captured verbatim instead.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid".into(),
            "ask".into(),
            "right".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_price(15_000, 10),
            dv_price(15_100, 10),
            dv_number(i64::MAX),
        ])],
    };
    assert_eq!(
        parse_quote_ticks(&table).unwrap_err(),
        DecodeError::UnknownEnumVariant {
            field: "right",
            raw: i64::MAX.to_string(),
        }
    );
}

// ─────────── Generic integer wire arms: int64 overflow guards ───────────
//
// Round-4 hardened the date and right surface against `*n as i32` narrowing
// but the generic `row_number` helper plus the `eod_num` generator template
// still cast wire `int64` payloads through `as i32` with no width check.
// `DataValue.number` is wire-typed `int64`, so a payload like
// `4_329_167_296` (== `(1 << 32) + 34_200_000`) truncated cleanly into a
// plausible-looking `ms_of_day` / `sequence` / `size` / `exchange` / bid/ask
// size / EOD integer value and silently corrupted the destination field
// across the whole non-EOD generator surface (via `opt_number`) plus every
// `eod_num` column. Both helpers now route through `i32::try_from` first
// and raise `DecodeError::NumericOverflow` with the raw `int64` captured
// verbatim.

#[test]
fn row_number_rejects_int64_above_i32_range() {
    // 4_294_967_296 == (1 << 32). Outside i32 range; the low 32 bits decode
    // to 0, which would silently zero out the destination field if the wire
    // value reached the parser narrowed via `*n as i32`.
    let row = row_of(vec![dv_number(4_294_967_296)]);
    assert_eq!(
        row_number(&row, 0),
        Err(DecodeError::NumericOverflow {
            raw: "4294967296".into(),
        })
    );
}

#[test]
fn row_number_rejects_int64_max() {
    let row = row_of(vec![dv_number(i64::MAX)]);
    assert_eq!(
        row_number(&row, 0),
        Err(DecodeError::NumericOverflow {
            raw: i64::MAX.to_string(),
        })
    );
}

#[test]
fn row_number_accepts_value_inside_i32_range() {
    // Regression smoke: a real `ms_of_day` value at 09:30:00 ET must
    // still decode bit-exact after the bounds check lands.
    let row = row_of(vec![dv_number(34_200_000)]);
    assert_eq!(row_number(&row, 0).unwrap(), Some(34_200_000));
}

#[test]
fn row_number_accepts_i32_max_and_min() {
    // The i32 boundary values themselves must round-trip — the
    // bounds check is inclusive on both ends.
    let max_row = row_of(vec![dv_number(i64::from(i32::MAX))]);
    assert_eq!(row_number(&max_row, 0).unwrap(), Some(i32::MAX));
    let min_row = row_of(vec![dv_number(i64::from(i32::MIN))]);
    assert_eq!(row_number(&min_row, 0).unwrap(), Some(i32::MIN));
}

#[test]
fn parse_trade_ticks_rejects_overflowing_ms_of_day() {
    // The `ms_of_day` column flows through `row_number` via the required
    // arm of the generated `parse_trade_ticks`. A wire payload like
    // `(1 << 32) + 34_200_000` previously truncated to a real-looking
    // `34_200_000` and corrupted every trade in the response; the bounds
    // check now surfaces the raw `int64` verbatim.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "price".into()],
        data_table: vec![row_of(vec![dv_number(4_329_167_296), dv_price(15_000, 10)])],
    };
    assert_eq!(
        parse_trade_ticks(&table).unwrap_err(),
        DecodeError::NumericOverflow {
            raw: "4329167296".into(),
        }
    );
}

#[test]
fn parse_trade_ticks_rejects_overflowing_sequence() {
    // `sequence` is an optional column that flows through `opt_number`
    // and from there through `row_number`. `i64::MAX` exercises the
    // far-end of the wire-int64 range against the same code path.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "sequence".into(), "price".into()],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_number(i64::MAX),
            dv_price(15_000, 10),
        ])],
    };
    assert_eq!(
        parse_trade_ticks(&table).unwrap_err(),
        DecodeError::NumericOverflow {
            raw: i64::MAX.to_string(),
        }
    );
}

#[test]
fn parse_quote_ticks_rejects_overflowing_bid_size() {
    // `bid_size` is an `i32` column on `QuoteTick`. A trillion-share
    // payload is the canonical "wire drifted to int64" failure mode —
    // without the bounds check it would silently truncate via the
    // `opt_number -> row_number` path and ship a fabricated size.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "bid_size".into(),
            "bid".into(),
            "ask".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_number(1_000_000_000_000),
            dv_price(15_000, 10),
            dv_price(15_100, 10),
        ])],
    };
    assert_eq!(
        parse_quote_ticks(&table).unwrap_err(),
        DecodeError::NumericOverflow {
            raw: "1000000000000".into(),
        }
    );
}

#[test]
fn parse_eod_ticks_rejects_overflowing_numeric_field() {
    // The `eod_num` generator helper covers every i32 EOD column
    // (ms_of_day, ms_of_day2, bid/ask sizes, bid/ask exchanges,
    // bid/ask conditions). Drive an overflow through `ms_of_day` so
    // the eod_num Number arm runs against the bounds check.
    let table = proto::DataTable {
        headers: vec!["ms_of_day".into(), "open".into()],
        data_table: vec![row_of(vec![dv_number(4_329_167_296), dv_number(15_000)])],
    };
    assert_eq!(
        parse_eod_ticks(&table).unwrap_err(),
        DecodeError::NumericOverflow {
            raw: "4329167296".into(),
        }
    );
}

#[test]
fn parse_trade_ticks_smoke_with_in_range_integers() {
    // Positive smoke: every i32 column inside the supported range
    // must still decode unchanged after the bounds check lands. This
    // is the regression sentinel for the generic Number arm.
    let table = proto::DataTable {
        headers: vec![
            "ms_of_day".into(),
            "sequence".into(),
            "size".into(),
            "exchange".into(),
            "price".into(),
            "date".into(),
        ],
        data_table: vec![row_of(vec![
            dv_number(34_200_000),
            dv_number(1),
            dv_number(100),
            dv_number(4),
            dv_price(15_000, 10),
            dv_number(20_240_301),
        ])],
    };
    let ticks = parse_trade_ticks(&table).unwrap();
    assert_eq!(ticks.len(), 1);
    let t = &ticks[0];
    assert_eq!(t.ms_of_day, 34_200_000);
    assert_eq!(t.sequence, 1);
    assert_eq!(t.size, 100);
    assert_eq!(t.exchange, 4);
    assert_eq!(t.date, 20_240_301);
}
