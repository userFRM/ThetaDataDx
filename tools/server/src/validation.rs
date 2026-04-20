//! Server-side input length caps for REST + WebSocket inputs.
//!
//! These checks are the first line of defense against memory-DoS from
//! malicious or broken clients (e.g. `?root=<1 MB string>`). They complement
//! the semantic validators in `thetadatadx::validate` (which check format,
//! not bounded size) and sit on both the REST `build_endpoint_args` path
//! and the WebSocket `subscribe` command path.
//!
//! # Design
//!
//! Caps are deliberately tighter than the real ThetaData upstream will
//! ever accept (ticker symbols are 1..=8 alphanumerics in practice, but
//! we allow up to 16 to leave headroom for upstream changes). The goal
//! is to reject garbage cheaply before any allocation flows into
//! `EndpointArgs::insert_raw`, proto builders, or `Contract::stock`.
//!
//! # Errors
//!
//! Every cap violation returns `ValidationError`, which the REST handler
//! surfaces as `400 Bad Request` (never `500 Internal Server Error`).
//! The WebSocket handler renders it as `REQ_RESPONSE { response: ERROR }`.

use thetadatadx::endpoint::EndpointError;

// ---------------------------------------------------------------------------
//  Length caps
// ---------------------------------------------------------------------------
//
// Constants are per-field so reviewers can verify each cap against the
// upstream ThetaData schema without hunting through the function body.

/// Ticker symbol / root: CBOE / OPRA symbols are <= 6 chars; 16 is ample
/// headroom without allowing memory-DoS.
pub const MAX_SYMBOL_LEN: usize = 16;

/// Comma-separated symbol list (`?roots=AAPL,MSFT,...`). 16 symbols * 16
/// chars each, plus 15 separators = 271. Round to 512 for safety.
pub const MAX_SYMBOLS_LEN: usize = 512;

/// YYYYMMDD -- exactly 8 digits, but allow slight flex for ISO variants
/// handled by `validate_expiration` (e.g. `YYYY-MM-DD` = 10 chars).
pub const MAX_DATE_LEN: usize = 10;

/// Max decimal strike-price string width. Largest real strike is well
/// under 10 digits.
pub const MAX_STRIKE_LEN: usize = 10;

/// Option right: "C" / "P" / "call" / "put" / "both" / "*".
pub const MAX_RIGHT_LEN: usize = 8;

/// Interval string: millis or shorthand ("500ms", "1m"). Cap generously.
pub const MAX_INTERVAL_LEN: usize = 16;

/// Venue / exchange code. SIP / MIC / OPRA codes are <= 4 chars; headroom
/// to 8 for future multi-part identifiers.
pub const MAX_VENUE_LEN: usize = 8;

/// Generic fallback for any string param not explicitly matched above.
/// Covers request-type strings, year strings, etc. Large enough for any
/// realistic upstream field, small enough to kill memory-DoS.
pub const MAX_GENERIC_LEN: usize = 64;

// ---------------------------------------------------------------------------
//  Error type
// ---------------------------------------------------------------------------

/// Input length violation. Carries both the offending field name and the
/// observed length so error responses are actionable.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: &'static str,
    pub message: String,
}

impl ValidationError {
    fn too_long(field: &'static str, observed: usize, limit: usize) -> Self {
        Self {
            field,
            message: format!("'{field}' exceeds maximum length of {limit} bytes (got {observed})"),
        }
    }

    fn invalid_content(field: &'static str, detail: &str) -> Self {
        Self {
            field,
            message: format!("'{field}' {detail}"),
        }
    }
}

impl From<ValidationError> for EndpointError {
    fn from(err: ValidationError) -> Self {
        EndpointError::InvalidParams(err.message)
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ValidationError {}

// ---------------------------------------------------------------------------
//  Shared helpers
// ---------------------------------------------------------------------------

/// Reject any string containing ASCII / Unicode control characters.
///
/// Addresses two concrete attack shapes:
///
/// 1. **Null-byte / control-char smuggling** into downstream consumers
///    (e.g. `Contract::stock("AAPL\0")`, OPRA symbol tables, JSON error
///    echoes) — control characters have no legitimate place in any
///    ThetaData query parameter.
/// 2. **ANSI-escape injection** into server logs when the error message
///    echoes the user-supplied value. Escape sequences in a terminal-
///    rendered log can move the cursor, clear the screen, or spoof
///    following lines.
///
/// The check runs AFTER the length cap (cheaper to reject a 1 MB payload
/// on length than to scan it for control chars) and BEFORE the value
/// flows into any downstream allocator or JSON builder. Legitimate
/// ThetaData inputs are pure ASCII alphanumerics plus `.,-_*`; none of
/// those are classified as control characters by `char::is_control`.
pub fn ensure_no_control_chars(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.chars().any(|c| c.is_control()) {
        return Err(ValidationError::invalid_content(
            field,
            "contains control characters",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
//  Field-specific validators
// ---------------------------------------------------------------------------

/// Validate a ticker symbol (`?root=AAPL`) against the server-side length cap.
///
/// Only checks bounded size. Format validity (alphanumeric, non-empty) is
/// enforced downstream by `thetadatadx::validate::validate_symbol`.
pub fn validate_symbol(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::invalid_content(field, "must be non-empty"));
    }
    if value.len() > MAX_SYMBOL_LEN {
        return Err(ValidationError::too_long(
            field,
            value.len(),
            MAX_SYMBOL_LEN,
        ));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Validate a comma-separated symbols list (`?roots=AAPL,MSFT,...`).
pub fn validate_symbols_list(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() > MAX_SYMBOLS_LEN {
        return Err(ValidationError::too_long(
            field,
            value.len(),
            MAX_SYMBOLS_LEN,
        ));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Validate a date or expiration string. Accepts both `YYYYMMDD` (8 chars)
/// and `YYYY-MM-DD` (10 chars); stricter format validation happens in
/// `thetadatadx::validate::validate_date` / `validate_expiration`.
pub fn validate_date(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() > MAX_DATE_LEN {
        return Err(ValidationError::too_long(field, value.len(), MAX_DATE_LEN));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Validate a strike-price string (decimal or `*`).
pub fn validate_strike(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() > MAX_STRIKE_LEN {
        return Err(ValidationError::too_long(
            field,
            value.len(),
            MAX_STRIKE_LEN,
        ));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Validate an option `right` string.
pub fn validate_right(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::invalid_content(field, "must be non-empty"));
    }
    if value.len() > MAX_RIGHT_LEN {
        return Err(ValidationError::too_long(field, value.len(), MAX_RIGHT_LEN));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Validate an interval string (e.g. `"60000"` or `"1m"`).
pub fn validate_interval(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() > MAX_INTERVAL_LEN {
        return Err(ValidationError::too_long(
            field,
            value.len(),
            MAX_INTERVAL_LEN,
        ));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Validate a venue / exchange code.
pub fn validate_venue(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.len() > MAX_VENUE_LEN {
        return Err(ValidationError::too_long(field, value.len(), MAX_VENUE_LEN));
    }
    ensure_no_control_chars(value, field)?;
    Ok(())
}

/// Generic fallback length check for any string param not matched by a
/// more specific validator. Use this for request-type strings, free-form
/// `?ivl=...` values past interval shorthand, etc.
/// Length-cap an unknown query parameter at 64 bytes. Keeps the
/// caller-supplied parameter name in the error message so operators can
/// identify which field triggered the rejection. The struct's `field`
/// label stays `"parameter"` (a 'static alias for unknown names); the
/// real name appears in `message` so the HTTP 400 body reads e.g.
/// `"'foobar' exceeds maximum length of 64 bytes (got 9001)"`.
///
/// The `param_name` is sanitized before being echoed back: only ASCII
/// alphanumerics, `_`, and `-` survive. Anything else (control chars,
/// ANSI escapes, non-ASCII) is stripped so the error body cannot carry
/// attacker-controlled bytes into operator terminals or aggregated logs.
/// A legitimate query-parameter name is already a C-identifier-shaped
/// string; filtering is a no-op for valid inputs.
pub fn validate_generic_named(value: &str, param_name: &str) -> Result<(), ValidationError> {
    if value.len() > MAX_GENERIC_LEN {
        let safe_name: String = sanitize_param_name(param_name);
        return Err(ValidationError {
            field: "parameter",
            message: format!(
                "'{safe_name}' exceeds maximum length of {MAX_GENERIC_LEN} bytes \
                 (got {observed})",
                observed = value.len()
            ),
        });
    }
    // Control-char rejection on generic params. We cannot anchor the error
    // on the untrusted `param_name`, so echo the sanitized form.
    if value.chars().any(|c| c.is_control()) {
        let safe_name: String = sanitize_param_name(param_name);
        return Err(ValidationError {
            field: "parameter",
            message: format!("'{safe_name}' contains control characters"),
        });
    }
    Ok(())
}

/// Filter a caller-supplied query-parameter name down to a safe set before
/// echoing it in an error message. Used by `validate_generic_named` to
/// neutralize ANSI / control sequences in attacker-controlled keys.
fn sanitize_param_name(param_name: &str) -> String {
    param_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

// ---------------------------------------------------------------------------
//  Unified entry point for REST query-param validation
// ---------------------------------------------------------------------------

/// Dispatch a raw query-param to the appropriate length validator based on
/// the known parameter name. Unrecognized names fall back to the generic
/// 64-byte cap.
///
/// This is called from `handler::build_endpoint_args` BEFORE the raw value
/// is parsed into an `EndpointArgValue` so we bound memory + CPU before
/// anything expensive happens.
pub fn validate_query_param(name: &str, value: &str) -> Result<(), ValidationError> {
    match name {
        "root" | "symbol" | "ticker" => validate_symbol(value, static_name(name)),
        "roots" | "symbols" | "tickers" => validate_symbols_list(value, static_name(name)),
        "exp" | "expiration" | "expiry" => validate_date(value, static_name(name)),
        "date" | "start_date" | "end_date" | "trade_date" => {
            validate_date(value, static_name(name))
        }
        "strike" => validate_strike(value, static_name(name)),
        "right" => validate_right(value, static_name(name)),
        "ivl" | "interval" => validate_interval(value, static_name(name)),
        "venue" | "exchange" => validate_venue(value, static_name(name)),
        _ => validate_generic_named(value, name),
    }
}

/// Pick a `'static` label for an error message. We only ever match on the
/// known param names above, so returning a short static alias for each one
/// keeps `ValidationError::field` allocation-free and avoids interning the
/// caller's borrowed string.
fn static_name(name: &str) -> &'static str {
    match name {
        "root" => "root",
        "symbol" => "symbol",
        "ticker" => "ticker",
        "roots" => "roots",
        "symbols" => "symbols",
        "tickers" => "tickers",
        "exp" => "exp",
        "expiration" => "expiration",
        "expiry" => "expiry",
        "date" => "date",
        "start_date" => "start_date",
        "end_date" => "end_date",
        "trade_date" => "trade_date",
        "strike" => "strike",
        "right" => "right",
        "ivl" => "ivl",
        "interval" => "interval",
        "venue" => "venue",
        "exchange" => "exchange",
        _ => "parameter",
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_rejects_oversized() {
        let big = "A".repeat(MAX_SYMBOL_LEN + 1);
        let err = validate_symbol(&big, "root").unwrap_err();
        assert_eq!(err.field, "root");
        assert!(err.message.contains("exceeds maximum length"));
    }

    #[test]
    fn symbol_rejects_empty() {
        let err = validate_symbol("", "root").unwrap_err();
        assert!(err.message.contains("non-empty"));
    }

    #[test]
    fn symbol_accepts_realistic() {
        for s in ["AAPL", "MSFT", "BRK.A", "SPY", "A"] {
            validate_symbol(s, "root").expect(s);
        }
    }

    #[test]
    fn date_rejects_long_payload() {
        let big = "1".repeat(MAX_DATE_LEN + 1);
        assert!(validate_date(&big, "date").is_err());
    }

    #[test]
    fn date_accepts_iso_and_compact() {
        validate_date("20260420", "date").unwrap();
        validate_date("2026-04-20", "date").unwrap();
    }

    #[test]
    fn right_rejects_oversized() {
        let big = "C".repeat(MAX_RIGHT_LEN + 1);
        assert!(validate_right(&big, "right").is_err());
    }

    #[test]
    fn strike_rejects_oversized() {
        let big = "9".repeat(MAX_STRIKE_LEN + 1);
        assert!(validate_strike(&big, "strike").is_err());
    }

    #[test]
    fn generic_rejects_megabyte_payload() {
        let big = "x".repeat(MAX_GENERIC_LEN + 1);
        let err = validate_generic_named(&big, "foobar").expect_err("oversized must reject");
        assert_eq!(err.field, "parameter");
        assert!(
            err.message.contains("'foobar'"),
            "error must surface the real param name, got: {}",
            err.message
        );
    }

    #[test]
    fn query_param_dispatch_routes_to_right_validator() {
        // 1 MB root -- the concrete DoS scenario called out in the audit.
        let mb = "A".repeat(1024 * 1024);
        let err = validate_query_param("root", &mb).unwrap_err();
        assert_eq!(err.field, "root");
    }

    #[test]
    fn endpoint_error_conversion_preserves_message() {
        let v = ValidationError {
            field: "root",
            message: "bad root".to_string(),
        };
        let ee: EndpointError = v.into();
        match ee {
            EndpointError::InvalidParams(m) => assert_eq!(m, "bad root"),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    //  H1 — control-character rejection on every typed validator
    // -----------------------------------------------------------------------

    #[test]
    fn symbol_rejects_null_byte() {
        // `AAPL\0` is 5 bytes and passes the length cap; only the control-
        // char check can catch the null-byte smuggling path into Contract.
        let err =
            validate_symbol("AAPL\0", "root").expect_err("symbol containing NUL must be rejected");
        assert!(err.message.contains("control"));
    }

    #[test]
    fn symbol_rejects_ansi_escape() {
        let err = validate_symbol("A\x1b[31m", "root").expect_err("ANSI escape must be rejected");
        assert!(err.message.contains("control"));
    }

    #[test]
    fn symbols_list_rejects_control_char() {
        assert!(validate_symbols_list("AAPL,MSFT\n,TSLA", "roots").is_err());
    }

    #[test]
    fn date_rejects_control_char() {
        assert!(validate_date("2026\r\n01", "date").is_err());
    }

    #[test]
    fn strike_rejects_control_char() {
        assert!(validate_strike("123\x01", "strike").is_err());
    }

    #[test]
    fn right_rejects_control_char() {
        assert!(validate_right("C\x7f", "right").is_err());
    }

    #[test]
    fn interval_rejects_control_char() {
        assert!(validate_interval("1m\x0b", "ivl").is_err());
    }

    #[test]
    fn venue_rejects_control_char() {
        assert!(validate_venue("OPRA\t", "venue").is_err());
    }

    #[test]
    fn generic_rejects_control_char() {
        let err = validate_generic_named("ok\x00payload", "foo").expect_err("NUL must be rejected");
        assert!(err.message.contains("control"));
    }

    // -----------------------------------------------------------------------
    //  M1 — echoed parameter name is sanitized
    // -----------------------------------------------------------------------

    #[test]
    fn generic_sanitizes_param_name_on_length_error() {
        let big = "x".repeat(MAX_GENERIC_LEN + 1);
        // `\x1b` (ESC), `\n`, `[`, `]`, `;` are all non-allowed characters
        // for param names: the filter strips anything that isn't ASCII
        // alphanumeric / `_` / `-`. The legitimate 'name' bytes that remain
        // are kept so operators can still identify the field.
        let attacker_key = "bad\x1b\nname;DROP";
        let err = validate_generic_named(&big, attacker_key).unwrap_err();
        // Control chars, ANSI escape byte, newline, semicolons must not
        // survive the echo.
        assert!(
            !err.message.contains('\x1b'),
            "ANSI escape leaked: {}",
            err.message
        );
        assert!(
            !err.message.contains('\n'),
            "newline leaked: {}",
            err.message
        );
        assert!(
            !err.message.contains(';'),
            "semicolon leaked: {}",
            err.message
        );
        // Legitimate prefix / suffix survive: 'badnameDROP' (contiguous
        // after stripping the disallowed bytes in between).
        assert!(
            err.message.contains("badnameDROP"),
            "sanitized name missing: {}",
            err.message
        );
    }

    #[test]
    fn generic_sanitizes_param_name_on_control_char_error() {
        let attacker_key = "\x1b[2J\x1b[H../etc";
        let err = validate_generic_named("ok\x00bad", attacker_key).unwrap_err();
        assert!(!err.message.contains('\x1b'));
        assert!(!err.message.contains('/'));
        assert!(!err.message.contains('.'));
    }

    #[test]
    fn sanitize_param_name_preserves_legitimate_names() {
        assert_eq!(sanitize_param_name("my_param"), "my_param");
        assert_eq!(sanitize_param_name("my-param-2"), "my-param-2");
        assert_eq!(sanitize_param_name("MIXED_case123"), "MIXED_case123");
    }
}
