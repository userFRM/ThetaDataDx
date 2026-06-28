//! Header alias table and lookup helper.
//!
//! v3 MDDS uses different column names than the tick schema. The
//! `HEADER_ALIASES` table maps schema names to their v3 equivalents
//! so generated and hand-written parsers work with both the schema
//! and v3 wire payloads.

/// Header aliases: v3 MDDS uses different column names than the tick schema.
/// This maps schema names to their v3 equivalents so parsers work with both.
///
/// Validated against a real v3 MDDS response capture (see
/// `tests/fixtures/captures/`). Each entry is `(schema_name, server_name)`:
/// `find_header("ms_of_day", h)` returns the index of the first matching
/// server column in `h`.
pub(crate) const HEADER_ALIASES: &[(&str, &str)] = &[
    // Generic time column: MDDS sends a proto `Timestamp`, the tick schema
    // models it as an i32 ms-of-day. `row_number` handles the conversion.
    ("ms_of_day", "timestamp"),
    // Combined trade + quote responses split the two time columns into
    // `trade_timestamp` (the trade side → `ms_of_day`) and `quote_timestamp`
    // (the quote side → `quote_ms_of_day`). Without these aliases the
    // `TradeQuoteTick` parser falls through the required-header guard and
    // produces an empty Vec on ~1M-row responses (P11).
    ("ms_of_day", "trade_timestamp"),
    ("quote_ms_of_day", "quote_timestamp"),
    // The EOD schema columns carry the v3 wire names directly
    // (`created` / `last_trade`, capture-verified); `timestamp` /
    // `timestamp2` are the legacy server spellings observed on older
    // terminal builds.
    ("created", "timestamp"),
    ("last_trade", "timestamp2"),
    ("date", "timestamp"),
    ("date", "created"),
    ("date", "trade_timestamp"),
    // option_list_contracts returns "symbol" where the schema says "root"
    ("root", "symbol"),
    // v3 uses "implied_vol" where the schema says "implied_volatility".
    // The same naming applies to the bid/ask-side IV columns emitted by
    // `option_history_greeks_implied_volatility`: server header
    // `bid_implied_vol` / `ask_implied_vol`, schema field
    // `bid_implied_volatility` / `ask_implied_volatility`.
    ("implied_volatility", "implied_vol"),
    ("bid_implied_volatility", "bid_implied_vol"),
    ("ask_implied_volatility", "ask_implied_vol"),
    // The vendor's per-order Greeks endpoints (`option_*_greeks_*_order`)
    // and the `_greeks_all` / `_greeks_eod` endpoints publish the
    // underlying snapshot timestamp as `underlying_timestamp`. The tick
    // schema models it as `underlying_ms_of_day` so the wire conversion
    // (Timestamp -> ms-of-day) flows through the standard `row_number`
    // path without a per-tick parser branch.
    ("underlying_ms_of_day", "underlying_timestamp"),
];

/// Helper: find a column index by name, with alias fallback.
///
/// The v3 MDDS server uses `timestamp` where the tick schema says `ms_of_day`.
/// This function checks the primary name first, then falls back to known aliases.
///
/// Returns `None` silently when the header is absent — required-header
/// guards in the generated parsers surface a typed
/// [`crate::error::Error::MissingRequiredHeader`] for the must-have columns;
/// optional columns missing from a subset response (e.g.
/// `option_snapshot_greeks_third_order` returning only the third-order Greek
/// columns from the `GreeksTick` union schema) are by design. Header drift
/// can be observed at the `trace` level via `RUST_LOG=thetadatadx=trace`.
///
/// # Lenient subset-column handling
///
/// On the NBBO endpoints the decoder accepts both the full 11-field
/// header
/// (`[ms_of_day, bid_size, bid_exchange, bid, bid_condition, ask_size,
/// ask_exchange, ask, ask_condition, date]`) and a 6-field subset
/// (`[ms_of_day, bid_size, bid, ask_size, ask, date]`). The
/// `QuoteTick` schema declares `bid_exchange`, `bid_condition`,
/// `ask_exchange`, `ask_condition` as optional columns — when the
/// wire response omits them the generated parser skips their column
/// pass and the `0` seed stands. The lookup
/// tolerates subset NBBO layouts the upstream may emit for older
/// storage tiers; the regression test
/// `tests::quote_tick_decodes_legacy_six_field_shape_with_zero_fill`
/// in this module pins that behaviour.
pub(crate) fn find_header(headers: &[&str], name: &str) -> Option<usize> {
    // Try exact match first.
    if let Some(pos) = headers.iter().position(|&s| s == name) {
        return Some(pos);
    }
    // Try aliases.
    for &(schema_name, server_name) in HEADER_ALIASES {
        if name == schema_name {
            if let Some(pos) = headers.iter().position(|&s| s == server_name) {
                return Some(pos);
            }
        }
    }
    tracing::trace!(
        header = name,
        "column header not present in DataTable (optional or subset response)"
    );
    None
}
