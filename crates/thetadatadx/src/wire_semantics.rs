//! Shared wire-level canonicalization rules.
//!
//! Consumed by:
//! - runtime request building in `direct.rs`
//! - build-time mode collapsing in `build_support/endpoints/modes.rs` (via
//!   `#[path]` reuse of this file)

/// Stock endpoints default an omitted `venue` to NQB.
pub(crate) const DEFAULT_STOCK_VENUE: &str = "nqb";

/// Lowercase string expected by the MDDS server (`"call"` / `"put"` /
/// `"both"`).
///
/// # Panics
///
/// Panics if `right` is not one of the accepted SDK surface forms.
pub(crate) fn normalize_right(right: &str) -> String {
    tdbe::right::parse_right(right)
        .unwrap_or_else(|err| panic!("{err}"))
        .as_mdds_str()
        .to_string()
}

/// Canonicalize the `expiration` parameter for the MDDS server.
///
/// Accepts the SDK's legacy `"0"` sentinel and the documented ISO-dashed
/// form, normalizing both to the wire vocabulary.
pub(crate) fn normalize_expiration(expiration: &str) -> String {
    match expiration {
        "0" => "*".to_string(),
        v if is_iso_date(v) => v.replace('-', ""),
        other => other.to_string(),
    }
}

/// Map the SDK `strike` vocabulary to the wire representation.
pub(crate) fn wire_strike_opt(strike: &str) -> Option<String> {
    if strike.is_empty() || strike == "*" || strike == "0" {
        None
    } else {
        Some(strike.to_string())
    }
}

/// Map the SDK `right` vocabulary to the wire representation.
pub(crate) fn wire_right_opt(right: &str) -> Option<String> {
    match tdbe::right::parse_right(right).unwrap_or_else(|err| panic!("{err}")) {
        tdbe::right::ParsedRight::Both => None,
        tdbe::right::ParsedRight::Call | tdbe::right::ParsedRight::Put => {
            Some(normalize_right(right))
        }
    }
}

/// Whether the string is `YYYY-MM-DD`.
pub(crate) fn is_iso_date(value: &str) -> bool {
    let mut parts = value.splitn(3, '-');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(y), Some(m), Some(d), None)
            if y.len() == 4
                && m.len() == 2
                && d.len() == 2
                && y.bytes().all(|b| b.is_ascii_digit())
                && m.bytes().all(|b| b.is_ascii_digit())
                && d.bytes().all(|b| b.is_ascii_digit())
    )
}
