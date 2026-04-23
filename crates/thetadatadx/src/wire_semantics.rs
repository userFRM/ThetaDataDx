//! Shared wire-level canonicalization rules.
//!
//! Consumed by:
//! - runtime request building in `mdds/endpoints.rs`
//! - build-time mode collapsing in `build_support/endpoints/modes.rs` (via
//!   `#[path]` reuse of this file)

// The `venue=nqb` default used to live here as a runtime constant applied
// at query-assembly time. In v8.0.10 the default moved into the SSOT
// (`endpoint_surface.toml` -> `stock_venue_filter.default = "nqb"`) so it
// flows through every emitted SDK builder uniformly. `modes.rs` now reads
// `param.default` directly; no runtime bridge needed.

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
///
/// The MDDS v3 server differentiates between an **absent** optional
/// `ContractSpec.strike` (per-strike enumeration -- slow path) and an
/// **explicit wildcard** `"*"` (chain-wide lookup -- fast path). The
/// request contract expects wildcard values to be populated literally:
/// the SDK-surface sentinels (`""`, `"*"`, `"0"`) all canonicalize to
/// the literal `"*"` string on the wire. Any other value forwards verbatim.
pub(crate) fn wire_strike_opt(strike: &str) -> Option<String> {
    if strike.is_empty() || strike == "*" || strike == "0" {
        Some("*".to_string())
    } else {
        Some(strike.to_string())
    }
}

/// Map the SDK `right` vocabulary to the wire representation.
///
/// Same fast-path / slow-path asymmetry as `wire_strike_opt`: an
/// unset `ContractSpec.right` triggers per-right enumeration on the
/// v3 server, while the explicit wire vocabulary (`"call"`, `"put"`,
/// `"both"`) hits the fast path. Vendor always populates; we mirror
/// that by always returning `Some(...)`.
///
/// # Errors
///
/// Returns the underlying `tdbe::right::parse_right` error if `right`
/// is not one of the accepted SDK surface forms.
pub(crate) fn wire_right_opt(right: &str) -> Result<Option<String>, tdbe::error::Error> {
    Ok(Some(
        tdbe::right::parse_right(right)?.as_mdds_str().to_string(),
    ))
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
