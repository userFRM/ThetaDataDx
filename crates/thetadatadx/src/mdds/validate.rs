//! Canonical parameter validation for `thetadatadx`.
//!
//! Every runtime check (date format, symbol format, interval legality,
//! option right, year, strike) lives here as the single source of truth.
//! Both the shared endpoint runtime ([`crate::mdds::endpoint_args`]) and
//! the MDDS client macros ([`crate::mdds::macros`]) delegate to these
//! functions.
//!
//! Build-time validators in `build_support/endpoints/` operate on the
//! TOML surface spec and proto schema — a fundamentally different
//! domain — so they remain separate.
//!
//! This module merges the previous top-level `crate::validate` and the
//! single-arg `crate::mdds::validate` adapter. The two-argument
//! canonical validators sit at the top; the single-arg adapters used
//! by the generated builder macros sit below.

use crate::error::Error;
use crate::mdds::endpoint_args::EndpointError;

// -- Canonical validators (two-arg, take a parameter name for diagnostics) --

/// Calendar-correct error message shared by every public date input.
///
/// Centralised so `YYYYMMDD` and `YYYY-MM-DD` paths cannot drift on
/// the bounds they document. Bounds match
/// [`tdbe::time::is_valid_gregorian_date`].
const GREGORIAN_BOUNDS_MSG: &str =
    "valid Gregorian date (year 1900-2100, month 1-12, day-of-month including 4/100/400 leap rule)";

/// Calendar-correct check shared by every public date entry point.
///
/// Both the `YYYYMMDD` and `YYYY-MM-DD` parsers feed the parsed
/// components through this single helper, so the surface accepts
/// exactly the same set of real dates regardless of which textual
/// form the caller used. The check itself lives in `tdbe::time` so
/// MDDS, FPSS, and `tdbe` consumers all use one canonical Gregorian
/// validator.
fn check_gregorian(
    year: i32,
    month: u32,
    day: u32,
    value: &str,
    param_name: &str,
) -> Result<(), EndpointError> {
    if tdbe::time::is_valid_gregorian_date(year, month, day) {
        Ok(())
    } else {
        Err(EndpointError::InvalidParams(format!(
            "'{param_name}' is not a {GREGORIAN_BOUNDS_MSG}, got: '{value}'"
        )))
    }
}

/// Parse `YYYY-MM-DD` into `(year, month, day)`.
///
/// Returns `None` if the textual shape is not exactly `4-2-2` digits.
/// Calendar correctness is left to [`check_gregorian`] so the shape
/// check and the calendar check stay independent of each other and
/// can be composed by callers that already know which form they hold.
fn parse_iso_date_components(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.splitn(3, '-');
    let (y, m, d) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(y), Some(m), Some(d), None) => (y, m, d),
        _ => return None,
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    if !y.bytes().all(|b| b.is_ascii_digit())
        || !m.bytes().all(|b| b.is_ascii_digit())
        || !d.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    // Parses cannot fail: every char is ASCII digit and the lengths
    // (4 / 2 / 2) fit comfortably in i32 / u32. The `ok()?` chain
    // keeps the function total in case prose-level reasoning ever
    // misses an edge case (e.g. a future tweak to accept negative
    // years).
    let year: i32 = y.parse().ok()?;
    let month: u32 = m.parse().ok()?;
    let day: u32 = d.parse().ok()?;
    Some((year, month, day))
}

pub(crate) fn validate_date(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be exactly 8 digits (YYYYMMDD), got: '{value}'"
        )));
    }
    // Shape passed; now apply the calendar check. Rejects the
    // `00000000` sentinel and impossible dates like `20260230` or
    // `19990431` that the shape-only check used to silently accept.
    let yyyymmdd: i32 = value.parse().map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be 8 digits (YYYYMMDD), got: '{value}'"
        ))
    })?;
    let year = yyyymmdd / 10_000;
    let month = ((yyyymmdd / 100) % 100) as u32;
    let day = (yyyymmdd % 100) as u32;
    check_gregorian(year, month, day, value, param_name)
}

/// Validate `expiration`: accepts `YYYY-MM-DD`, `YYYYMMDD`, `*`, or the
/// legacy `"0"` wildcard (translated to `*` in
/// [`crate::mdds::wire_semantics::normalize_expiration`]).
///
/// Both dated forms are calendar-checked — `2026-02-30`,
/// `2026-13-01`, and `2026-04-31` are rejected on every public input
/// regardless of which textual shape the caller used.
pub(crate) fn validate_expiration(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if matches!(value, "*" | "0") {
        return Ok(());
    }
    // ISO-dashed shape: parse components and run the same calendar
    // check the `YYYYMMDD` path uses, so the two surface forms accept
    // exactly the same set of dates. A shape mismatch falls through
    // to the digits-only path which may still recognise the input.
    if let Some((year, month, day)) = parse_iso_date_components(value) {
        return check_gregorian(year, month, day, value, param_name);
    }
    validate_date(value, param_name).map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be '*' (wildcard), '0' (legacy wildcard), 'YYYYMMDD', or 'YYYY-MM-DD', got: '{value}'"
        ))
    })
}

/// Validate the `strike` parameter.
///
/// Upstream documents `strike` as a decimal price string (e.g. `"550"`,
/// `"17.5"`) or `*` for all strikes, with `*` as the documented default.
/// We additionally accept `"0"` and the empty string as ergonomic
/// wildcard forms. Wildcards become proto-unset in
/// [`crate::mdds::wire_semantics::wire_strike_opt`] so the server applies
/// its documented default.
pub(crate) fn validate_strike(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.is_empty() || matches!(value, "*" | "0") {
        return Ok(());
    }
    match value.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => Ok(()),
        _ => Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be '*' (wildcard), '0' (legacy wildcard), or a positive decimal (e.g. '550' or '17.5'), got: '{value}'"
        ))),
    }
}

pub(crate) fn validate_symbol(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.is_empty() {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be non-empty"
        )));
    }
    Ok(())
}

/// The exact set of `interval` strings the v3 ThetaData server accepts.
///
/// Mirrors the upstream enum at
/// `https://docs.thetadata.us/operations/option_history_quote.html`.
/// The SDK additionally accepts decimal millisecond shorthand
/// (`"60000"`, `"300000"`, ...) and snaps it to the nearest preset via
/// [`crate::mdds::endpoints::normalize_interval`]; this validator
/// recognises both shapes so the CLI / MCP layer rejects garbage
/// before the gRPC dispatch.
const VALID_INTERVAL_PRESETS: &[&str] = &[
    "tick", "10ms", "100ms", "500ms", "1s", "5s", "10s", "15s", "30s", "1m", "5m", "10m", "15m",
    "30m", "1h",
];

pub(crate) fn validate_interval(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.is_empty() {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be a non-empty string from the upstream enum ({}) or a millisecond value (e.g. '60000'), got empty string",
            VALID_INTERVAL_PRESETS.join(", "),
        )));
    }
    if VALID_INTERVAL_PRESETS.contains(&value) {
        return Ok(());
    }
    if value.bytes().all(|b| b.is_ascii_digit()) {
        // Millisecond shorthand: `normalize_interval` will snap to the
        // nearest documented preset. Any positive integer is accepted
        // here; the snap range covers `0` (-> "tick") through `1h`.
        return Ok(());
    }
    Err(EndpointError::InvalidParams(format!(
        "'{param_name}' must be one of the upstream presets ({}) or a millisecond value (e.g. '60000'), got: '{value}'",
        VALID_INTERVAL_PRESETS.join(", "),
    )))
}

pub(crate) fn validate_right(value: &str, param_name: &str) -> Result<(), EndpointError> {
    // Delegate to the canonical parser so the accepted vocabulary stays
    // in one place. The endpoint layer does not distinguish
    // Call/Put/Both here -- per-endpoint logic in the MDDS client
    // decides whether `both` / `*` is meaningful -- so we only care
    // about "is this parseable at all".
    tdbe::right::parse_right(value).map(|_| ()).map_err(|_| {
        EndpointError::InvalidParams(format!(
            "'{param_name}' must be one of: 'call', 'put', 'both', 'C', 'P', '*' (case-insensitive), got: '{value}'"
        ))
    })
}

pub(crate) fn validate_year(value: &str, param_name: &str) -> Result<(), EndpointError> {
    if value.len() != 4 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EndpointError::InvalidParams(format!(
            "'{param_name}' must be exactly 4 digits (YYYY), got: '{value}'"
        )));
    }
    Ok(())
}

pub(crate) fn parse_symbols(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|symbol| !symbol.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn parse_bool(value: &str) -> Result<bool, &'static str> {
    if value.eq_ignore_ascii_case("true") || value == "1" {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") || value == "0" {
        Ok(false)
    } else {
        Err("accepted values are true, false, 1, or 0")
    }
}

// -- Single-arg adapter used by the generated builder macros --
//
// The two-arg canonical [`validate_date`] above takes a parameter name
// for diagnostics; the generated MDDS endpoint code emits
// `validate_date(&arg)?` and wants a one-arg form. The generated
// `mdds/endpoints.rs` calls into a `validate_date_required` helper
// re-exported below — same code path, single-arg shape.

/// Validate a date string for the generated builder macros.
///
/// Wraps the two-arg canonical [`validate_date`] in the single-arg
/// signature the `parsed_endpoint!` and streaming-builder macros
/// expect (the param name is implicit at the call site).
pub(super) fn validate_date_required(date: &str) -> Result<(), Error> {
    validate_date(date, "date").map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn validate_date_required_valid() {
        assert!(validate_date_required("20240101").is_ok());
        assert!(validate_date_required("20231231").is_ok());
        // Leap-year boundaries — the calendar check accepts these.
        assert!(validate_date_required("20240229").is_ok());
        assert!(validate_date_required("20000229").is_ok());
    }

    #[test]
    fn validate_date_required_rejects_impossible_calendar_dates() {
        // The exact garbage shapes that shape-only validation used to
        // silently accept:
        assert!(validate_date_required("00000000").is_err());
        assert!(validate_date_required("20260230").is_err()); // Feb 30
        assert!(validate_date_required("19990431").is_err()); // Apr 31
        assert!(validate_date_required("20231300").is_err()); // month 13
        assert!(validate_date_required("19000229").is_err()); // /100 non-leap
        assert!(validate_date_required("18991231").is_err()); // year < 1900
        assert!(validate_date_required("21010101").is_err()); // year > 2100
    }

    fn assert_validate_date_required_err(input: &str) {
        match validate_date_required(input) {
            // `validate_date` returns `EndpointError::InvalidParams`,
            // which converts to `Error::Config` (see `impl
            // From<EndpointError> for Error` in `mdds::endpoint_args`).
            Err(Error::Config { message, .. }) => {
                assert!(
                    message.contains("date"),
                    "error message should name the 'date' param, got {message:?}"
                );
            }
            other => panic!("expected Error::Config {{ .. }} for input {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn validate_date_required_invalid() {
        // Too short
        assert_validate_date_required_err("2024010");
        // Too long
        assert_validate_date_required_err("202401011");
        // Contains non-digit
        assert_validate_date_required_err("2024-101");
        assert_validate_date_required_err("2024Jan1");
        // Empty
        assert_validate_date_required_err("");
        // Whitespace
        assert_validate_date_required_err("2024 101");
    }

    #[test]
    fn expiration_accepts_documented_vocab_and_rejects_garbage() {
        for good in ["*", "0", "20260417", "2026-04-17"] {
            assert!(validate_expiration(good, "expiration").is_ok(), "{good}");
        }
        for bad in [
            "",
            "abc",
            "202604175",
            "2026/04/17",
            // Calendar-impossible dates on the digits-only path. The
            // Gregorian check rejects them on every public input.
            "20260230",
            "19990431",
            "00000000",
        ] {
            assert!(validate_expiration(bad, "expiration").is_err(), "{bad}");
        }
    }

    #[test]
    fn expiration_iso_dashed_form_enforces_calendar_bounds() {
        // The previous code path checked only the textual shape of
        // `YYYY-MM-DD` and accepted calendar-impossible inputs. Both
        // forms now flow through the same Gregorian validator, so
        // these inputs are rejected.
        for bad in [
            "2026-02-30", // Feb 30 — no such day
            "2026-13-01", // month 13 — out of range
            "2026-04-31", // Apr 31 — only 30 days
            "1899-12-31", // year < 1900
            "2101-01-01", // year > 2100
            "0000-00-00", // sentinel — every component invalid
            "1900-02-29", // year /100 non-leap
        ] {
            assert!(
                validate_expiration(bad, "expiration").is_err(),
                "expected calendar rejection on dashed form: {bad}"
            );
        }
        // Sanity: real leap-year boundaries still pass on the dashed
        // form. Two complementary cases.
        for good in [
            "2024-02-29", // /4 leap
            "2000-02-29", // /400 leap
        ] {
            assert!(
                validate_expiration(good, "expiration").is_ok(),
                "expected acceptance: {good}"
            );
        }
    }

    #[test]
    fn validate_date_yyyymmdd_path_rejects_same_impossibles_as_dashed_form() {
        // Symmetry guard: the two surface forms accept exactly the
        // same set of real dates. The dashed form is checked above;
        // the digits-only form rejects the corresponding inputs.
        for bad in [
            "20260230", "20261301", "20260431", "18991231", "21010101", "00000000", "19000229",
        ] {
            assert!(
                validate_date(bad, "date").is_err(),
                "expected calendar rejection on digits-only form: {bad}"
            );
        }
    }

    #[test]
    fn strike_accepts_wildcards_and_positive_decimals_and_rejects_garbage() {
        for good in ["*", "0", "", "550", "17.5", "0.5"] {
            assert!(validate_strike(good, "strike").is_ok(), "{good}");
        }
        for bad in ["abc", "-10", "1.5.3", "$500"] {
            assert!(validate_strike(bad, "strike").is_err(), "{bad}");
        }
    }

    #[test]
    fn interval_accepts_upstream_enum_and_ms_shorthand() {
        for good in [
            "tick", "10ms", "100ms", "500ms", "1s", "5s", "10s", "15s", "30s", "1m", "5m", "10m",
            "15m", "30m", "1h", "0", "60000", "300000",
        ] {
            assert!(validate_interval(good, "interval").is_ok(), "{good}");
        }
    }

    #[test]
    fn interval_rejects_garbage() {
        for bad in ["", "twosec", "2sec", "1minute", "-1", "1.5s", "1 s", "*"] {
            assert!(validate_interval(bad, "interval").is_err(), "{bad}");
        }
    }
}
