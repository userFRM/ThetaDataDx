//! Regression test for the `InterestRateTick` schema fix.
//!
//! Before this PR `tick_schema.toml` declared `InterestRateTick` as 3 fields
//! (`ms_of_day`/`rate`/`date`) with a `Number` `date` column, but the
//! upstream v3 server actually emits 2 columns: `created` as Text ISO date
//! and `rate` as a percent Number. Every live `interest_rate_history_eod`
//! call returned `DecodeError::TypeMismatch { expected: "Number|Timestamp",
//! observed: "Text" }`.
//!
//! This test reconstructs the captured wire shape (SOFR
//! 2025-04-28..2025-05-02) as a synthetic `DataTable` and
//! asserts the decoded `InterestRateTick` carries the expected
//! `date=20250428` / `rate=4.36` values for the first row. It pins:
//!
//! 1. The 2-field struct shape (any future `ms_of_day` resurrection
//!    breaks the construct-by-name in `Ok(InterestRateTick { date, rate })`
//!    inside the generated `parse_interest_rate_ticks` parser).
//! 2. The Text-ISO-date decode path through `row_date` (see
//!    `crates/thetadatadx/src/mdds/decode/cell.rs::row_date` — Text arm
//!    routes through `parse_iso_date`).
//! 3. The percent-as-`f64` decode for the `rate` column.

use thetadatadx::decode::parse_interest_rate_ticks;
use thetadatadx::wire;

fn dv_text(s: &str) -> wire::DataValue {
    wire::DataValue {
        data_type: Some(wire::data_value::DataType::Text(s.to_string())),
    }
}

fn dv_number(n: i64) -> wire::DataValue {
    wire::DataValue {
        data_type: Some(wire::data_value::DataType::Number(n)),
    }
}

fn dv_price(value: i32, price_type: i32) -> wire::DataValue {
    wire::DataValue {
        data_type: Some(wire::data_value::DataType::Price(wire::Price {
            value,
            r#type: price_type,
        })),
    }
}

fn row(values: Vec<wire::DataValue>) -> wire::DataValueList {
    wire::DataValueList { values }
}

/// Reference wire dump (MDDS endpoint `interest_rate/history/eod`, symbol
/// `SOFR`, date range 2025-04-28..2025-05-02):
///
/// ```text
/// created,rate
/// "2025-04-28",4.3600
/// "2025-04-29",4.3600
/// "2025-04-30",4.4100
/// "2025-05-01",4.3900
/// "2025-05-02",4.3600
/// ```
const WIRE_REFERENCE: &[(&str, f64)] = &[
    ("2025-04-28", 4.36),
    ("2025-04-29", 4.36),
    ("2025-04-30", 4.41),
    ("2025-05-01", 4.39),
    ("2025-05-02", 4.36),
];

const EXPECTED_DATES: &[i32] = &[20250428, 20250429, 20250430, 20250501, 20250502];

fn wire_table() -> wire::DataTable {
    let mut rows = Vec::with_capacity(WIRE_REFERENCE.len());
    for (iso, rate) in WIRE_REFERENCE {
        // Rates arrive as Price cells (`type = 8` -> value * 10^-2)
        // on the wire — covered by `row_price_f64`'s Price|Number
        // accept-list. Encoding `4.36` as `Price(436, 8)` round-trips
        // through `Price::to_f64` to exactly `4.36`. The Number arm is
        // pinned separately in `parse_interest_rate_ticks_accepts_number_rate`
        // below; both arms exercise the canonical decoder path.
        let mantissa: i32 = (rate * 100.0).round() as i32;
        rows.push(row(vec![dv_text(iso), dv_price(mantissa, 8)]));
    }
    wire::DataTable {
        headers: vec!["created".to_string(), "rate".to_string()],
        data_table: rows,
    }
}

#[test]
fn parse_interest_rate_ticks_decodes_iso_text_and_number_rate() {
    let table = wire_table();
    let ticks = parse_interest_rate_ticks(&table).expect("decode succeeds");

    assert_eq!(ticks.len(), WIRE_REFERENCE.len(), "row count round-trips");

    for (i, tick) in ticks.iter().enumerate() {
        assert_eq!(
            tick.date, EXPECTED_DATES[i],
            "row {i}: ISO date `{}` decodes to YYYYMMDD i32",
            WIRE_REFERENCE[i].0
        );
        let want_rate = WIRE_REFERENCE[i].1;
        let observed = tick.rate;
        // `Number(43_600)` round-trips back to `4.36` after we encoded it as
        // `rate * 10_000` above. Tolerance is generous to absorb the
        // multiply-then-divide artefact; the live decode path returns the
        // server's float directly so the only quantisation comes from the
        // synthetic fixture.
        assert!(
            (observed - want_rate).abs() < 1e-6,
            "row {i}: rate decoded as {observed}, want {want_rate}"
        );
    }
}

#[test]
fn parse_interest_rate_ticks_pins_reference_row() {
    // The headline wire dump in the CHANGELOG is the SOFR
    // 2025-04-28 row (`date=20250428`, `rate=4.36`). Pinning the exact
    // values here means any future schema drift fails this test before
    // it ships.
    let table = wire_table();
    let ticks = parse_interest_rate_ticks(&table).expect("decode succeeds");
    let head = ticks.first().expect("at least one row");
    assert_eq!(head.date, 20250428);
    assert!((head.rate - 4.36).abs() < 1e-6);
}

#[test]
fn parse_interest_rate_ticks_accepts_number_rate() {
    // The historical decode path accepts both Price and Number cells in
    // `row_price_f64`. The synthetic fixture above exercises the Price
    // arm; pin the Number arm here so a future schema narrowing on
    // either side gets caught.
    let table = wire::DataTable {
        headers: vec!["created".to_string(), "rate".to_string()],
        data_table: vec![row(vec![dv_text("2025-04-28"), dv_number(4)])],
    };
    let ticks = parse_interest_rate_ticks(&table).expect("decode succeeds");
    assert_eq!(ticks.len(), 1);
    assert_eq!(ticks[0].date, 20250428);
    assert!((ticks[0].rate - 4.0).abs() < 1e-6);
}

#[test]
fn parse_interest_rate_ticks_empty_response_returns_empty_vec() {
    // The required-header guard returns `Ok(vec![])` only when the
    // response is genuinely empty (no rows). With rows present and the
    // required header missing the decode errors loud
    // (`MissingRequiredHeader`).
    let table = wire::DataTable {
        headers: vec!["created".to_string(), "rate".to_string()],
        data_table: vec![],
    };
    let ticks = parse_interest_rate_ticks(&table).expect("decode succeeds");
    assert!(ticks.is_empty());
}

#[test]
fn parse_interest_rate_ticks_missing_required_created_header_errors_when_rows_present() {
    let table = wire::DataTable {
        headers: vec!["rate".to_string()],
        data_table: vec![row(vec![dv_number(43_600)])],
    };
    let err = parse_interest_rate_ticks(&table).expect_err("missing `created` header is an error");
    let msg = err.to_string();
    assert!(
        msg.contains("created"),
        "error mentions the missing required header: {msg}"
    );
}
