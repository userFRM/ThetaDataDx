//! Shared wire-level canonicalization rules.
//!
//! Consumed by:
//! - runtime request building in `mdds/endpoints.rs`
//! - build-time mode collapsing in `build_support/endpoints/modes.rs` (via
//!   `#[path]` reuse of this file)

// The `venue=nqb` default used to live here as a runtime constant applied
// at query-assembly time. The default now lives in the SSOT
// (`endpoint_surface.toml` -> `stock_venue_filter.default = "nqb"`) so it
// flows through every emitted SDK builder uniformly. `modes.rs` reads
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

/// Canonicalize a `date` / `start_date` / `end_date` parameter for the
/// MDDS server.
///
/// The validators accept both `YYYYMMDD` and the ISO-dashed
/// `YYYY-MM-DD` form (see `crate::mdds::validate::validate_date`); the
/// wire contract is the compact 8-digit form only. Mirrors the
/// ISO-normalization branch of [`normalize_expiration`]. Anything that
/// is not ISO-dashed forwards verbatim — by the time this runs, the
/// validators have already rejected malformed input on validated
/// paths, and unvalidated pass-through setters keep their existing
/// let-the-server-decide contract.
pub(crate) fn normalize_date(date: &str) -> String {
    if is_iso_date(date) {
        date.replace('-', "")
    } else {
        date.to_string()
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
/// Returns the parse error from [`thetadatadx::greeks::parse_right`] if
/// `right` is not one of the accepted SDK surface forms. The result is
/// the crate's public [`thetadatadx::Error`] so this helper resolves
/// identically in the library and in the `generate_sdk_surfaces` binary,
/// which share this file via `#[path]`.
pub(crate) fn wire_right_opt(right: &str) -> Result<Option<String>, thetadatadx::Error> {
    Ok(Some(
        thetadatadx::greeks::parse_right(right)?
            .as_mdds_str()
            .to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_date_strips_dashes_from_iso_form() {
        assert_eq!(normalize_date("2026-06-03"), "20260603");
        assert_eq!(normalize_date("2024-02-29"), "20240229");
    }

    #[test]
    fn normalize_date_passes_compact_form_through() {
        assert_eq!(normalize_date("20260603"), "20260603");
    }

    #[test]
    fn normalize_date_passes_non_date_shapes_through() {
        // Pass-through setters (`opt_str` builder fields) keep their
        // let-the-server-decide contract for anything that is not the
        // exact ISO shape; validated paths never reach here with
        // malformed input.
        for raw in ["2026-1-3", "garbage", "", "2026/06/03"] {
            assert_eq!(normalize_date(raw), raw);
        }
    }

    #[test]
    fn normalize_date_agrees_with_normalize_expiration_on_iso_input() {
        // The two canonicalizers share `is_iso_date`; pin the
        // agreement so the date and expiration wire forms cannot
        // drift.
        assert_eq!(
            normalize_date("2026-06-19"),
            normalize_expiration("2026-06-19")
        );
    }
}
