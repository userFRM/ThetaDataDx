//! Wire-format canonicalizers for MDDS request construction.
//!
//! Each function maps the SDK's user-facing vocabulary (permissive, multiple
//! equivalent spellings, legacy `"0"` sentinels) to the exact wire form the
//! v3 MDDS server accepts. The canonical logic lives in
//! [`crate::wire_semantics`]; this module is the MDDS-scoped facade consumed
//! by the `contract_spec!` macro and the generated endpoint bodies.

use crate::error::Error;

/// Canonicalize the `expiration` parameter for the v3 MDDS server.
///
/// Upstream's `openapiv3.yaml` documents the accepted expiration vocabulary
/// as `YYYY-MM-DD`, `YYYYMMDD`, or `*` for all expirations. We also accept
/// the legacy `"0"` sentinel and translate it to `"*"` here because the
/// server rejects `"0"` directly with `InvalidArgument -- Error parsing
/// expiration Cannot parse date string: 0`. ISO-dashed dates are
/// canonicalized to the compact `YYYYMMDD` form on the wire.
pub(crate) fn normalize_expiration(expiration: &str) -> String {
    crate::wire_semantics::normalize_expiration(expiration)
}

/// Map the SDK's `strike` surface vocabulary to the wire representation.
///
/// Upstream documents `strike` as an optional query parameter: if omitted,
/// the server applies its default (`*` wildcard). Our SDK-level surface
/// forces the caller to always pass a value, so we reinterpret the
/// wildcard sentinels (`*`, `0`, empty) as "leave the `ContractSpec.strike`
/// proto field unset" — same wire outcome as the upstream documented
/// omission. All other values forward verbatim.
pub(crate) fn wire_strike_opt(strike: &str) -> Option<String> {
    crate::wire_semantics::wire_strike_opt(strike)
}

/// Map the SDK's `right` surface vocabulary to the wire representation.
///
/// Upstream treats `right` as optional with `both` as the implicit "no
/// filter" state and `*` as its explicit wildcard. Both collapse to
/// proto-unset here so the server applies the documented default; single
/// sides (`C` / `P` / `call` / `put`, any case) normalize to `"call"` /
/// `"put"`. Any other input produces
/// [`Error::Config`](crate::error::Error::Config) carrying the message
/// from [`crate::right::parse_right`].
///
/// # Errors
///
/// Returns `Error::Config` if `right` is not one of the accepted SDK
/// surface forms.
pub(crate) fn wire_right_opt(right: &str) -> Result<Option<String>, Error> {
    crate::wire_semantics::wire_right_opt(right).map_err(Error::from)
}

/// Convert an interval to the format the MDDS gRPC server accepts.
///
/// Users can pass either:
/// - Milliseconds as a string: `"60000"`, `"300000"`, `"900000"`
/// - Shorthand directly: `"1m"`, `"5m"`, `"1h"`
///
/// The server accepts these specific presets:
/// `100ms`, `500ms`, `1s`, `5s`, `10s`, `15s`, `30s`, `1m`, `5m`, `10m`, `15m`, `30m`, `1h`
///
/// If milliseconds are passed, they're converted to the nearest matching preset.
/// If already a valid shorthand (contains 's', 'm', or 'h'), passed through as-is.
pub(super) fn normalize_interval(interval: &str) -> String {
    // If it already looks like shorthand (ends with s/m/h), pass through.
    if interval.ends_with('s') || interval.ends_with('m') || interval.ends_with('h') {
        return interval.to_string();
    }

    // Try parsing as milliseconds and convert to the nearest valid preset.
    //
    // Valid presets: 100ms, 500ms, 1s, 5s, 10s, 15s, 30s, 1m, 5m, 10m, 15m, 30m, 1h
    match interval.parse::<u64>() {
        Ok(ms) => match ms {
            0..=100 => "100ms".to_string(),
            101..=500 => "500ms".to_string(),
            501..=1000 => "1s".to_string(),
            1_001..=5_000 => "5s".to_string(),
            5_001..=10_000 => "10s".to_string(),
            10_001..=15_000 => "15s".to_string(),
            15_001..=30_000 => "30s".to_string(),
            30_001..=60_000 => "1m".to_string(),
            60_001..=300_000 => "5m".to_string(),
            300_001..=600_000 => "10m".to_string(),
            600_001..=900_000 => "15m".to_string(),
            900_001..=1_800_000 => "30m".to_string(),
            _ => "1h".to_string(),
        },
        // Not a number -- pass through and let the server decide.
        Err(_) => interval.to_string(),
    }
}

/// Convert `time_of_day` values into the canonical `HH:MM:SS.SSS` format.
///
/// ThetaData's v3 at-time endpoints expect a formatted ET wall-clock time such
/// as `"09:30:00.000"`. Older ThetaDataDx docs and examples used millisecond
/// strings like `"34200000"`. To preserve compatibility while aligning the
/// public contract, this helper accepts either form and normalizes to
/// `HH:MM:SS.SSS`.
///
/// Accepted inputs:
/// - Milliseconds from midnight as a decimal string: `"34200000"`
/// - Formatted times: `"09:30"`, `"09:30:00"`, `"09:30:00.000"`
///
/// Invalid or out-of-range values are passed through unchanged so the server
/// can return the canonical validation error.
pub(super) fn normalize_time_of_day(time_of_day: &str) -> String {
    if time_of_day.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(total_ms) = time_of_day.parse::<u64>() {
            if total_ms < 86_400_000 {
                let hours = total_ms / 3_600_000;
                let minutes = (total_ms % 3_600_000) / 60_000;
                let seconds = (total_ms % 60_000) / 1_000;
                let millis = total_ms % 1_000;
                return format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}");
            }
        }
        return time_of_day.to_string();
    }

    let mut parts = time_of_day.split(':');
    let Some(hours) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return time_of_day.to_string();
    };
    let Some(minutes) = parts.next().and_then(|part| part.parse::<u64>().ok()) else {
        return time_of_day.to_string();
    };
    let seconds_part = parts.next();
    if parts.next().is_some() {
        return time_of_day.to_string();
    }

    let (seconds, millis) = match seconds_part {
        None => (0, 0),
        Some(part) => match part.split_once('.') {
            Some((sec, frac)) => {
                let Some(seconds) = sec.parse::<u64>().ok() else {
                    return time_of_day.to_string();
                };
                let millis = match frac.len() {
                    1 => frac.parse::<u64>().ok().map(|value| value * 100),
                    2 => frac.parse::<u64>().ok().map(|value| value * 10),
                    3 => frac.parse::<u64>().ok(),
                    _ => None,
                };
                let Some(millis) = millis else {
                    return time_of_day.to_string();
                };
                (seconds, millis)
            }
            None => {
                let Some(seconds) = part.parse::<u64>().ok() else {
                    return time_of_day.to_string();
                };
                (seconds, 0)
            }
        },
    };

    if hours >= 24 || minutes >= 60 || seconds >= 60 || millis >= 1_000 {
        return time_of_day.to_string();
    }

    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// Helper: build a `proto::ContractSpec` from the four standard option params.
///
/// `symbol` and `expiration` are required by the v3 server. `strike` and
/// `right` are optional at the wire level (server applies wildcard defaults
/// when unset); our SDK surface promotes them to required positional args
/// and reinterprets wildcard sentinels as proto-unset via `wire_strike_opt`
/// and `wire_right_opt`.
macro_rules! contract_spec {
    ($symbol:expr, $expiration:expr, $strike:expr, $right:expr) => {
        Some(proto::ContractSpec {
            symbol: $symbol.to_string(),
            expiration: normalize_expiration(&$expiration),
            strike: wire_strike_opt(&$strike),
            right: wire_right_opt(&$right)?,
        })
    };
}

pub(super) use contract_spec;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_time_of_day_accepts_legacy_milliseconds() {
        assert_eq!(normalize_time_of_day("34200000"), "09:30:00.000");
    }

    #[test]
    fn normalize_time_of_day_accepts_short_formatted_values() {
        assert_eq!(normalize_time_of_day("09:30"), "09:30:00.000");
        assert_eq!(normalize_time_of_day("09:30:00"), "09:30:00.000");
        assert_eq!(normalize_time_of_day("09:30:00.5"), "09:30:00.500");
    }

    #[test]
    fn normalize_time_of_day_preserves_invalid_values_for_server_rejection() {
        assert_eq!(normalize_time_of_day("86400000"), "86400000");
        assert_eq!(normalize_time_of_day("09:61"), "09:61");
        assert_eq!(normalize_time_of_day("not-a-time"), "not-a-time");
    }

    // ── Wire-translation tests ────────────────────────────────────────────

    #[test]
    fn normalize_expiration_translates_legacy_zero_to_star() {
        assert_eq!(normalize_expiration("0"), "*");
    }

    #[test]
    fn normalize_expiration_passes_star_through() {
        assert_eq!(normalize_expiration("*"), "*");
    }

    #[test]
    fn normalize_expiration_passes_compact_date_through() {
        assert_eq!(normalize_expiration("20260417"), "20260417");
    }

    #[test]
    fn normalize_expiration_strips_iso_dashes() {
        assert_eq!(normalize_expiration("2026-04-17"), "20260417");
    }

    #[test]
    fn wire_strike_opt_treats_wildcards_as_unset() {
        assert_eq!(wire_strike_opt(""), None);
        assert_eq!(wire_strike_opt("0"), None);
        assert_eq!(wire_strike_opt("*"), None);
    }

    #[test]
    fn wire_strike_opt_forwards_real_strikes() {
        assert_eq!(wire_strike_opt("550"), Some("550".to_string()));
        assert_eq!(wire_strike_opt("17.5"), Some("17.5".to_string()));
    }

    #[test]
    fn wire_right_opt_treats_wildcards_as_unset() {
        assert_eq!(wire_right_opt("*").unwrap(), None);
        assert_eq!(wire_right_opt("both").unwrap(), None);
        assert_eq!(wire_right_opt("BOTH").unwrap(), None);
        assert_eq!(wire_right_opt("Both").unwrap(), None);
    }

    #[test]
    fn wire_right_opt_rejects_undocumented_forms() {
        // validate_right catches these earlier on the endpoint path;
        // wire_right_opt is the last defense and now returns an error
        // instead of panicking across FFI.
        assert!(wire_right_opt("").is_err());
    }

    #[test]
    fn wire_right_opt_rejects_zero_sentinel() {
        assert!(wire_right_opt("0").is_err());
    }

    #[test]
    fn wire_right_opt_forwards_single_sides_normalized() {
        assert_eq!(wire_right_opt("C").unwrap(), Some("call".to_string()));
        assert_eq!(wire_right_opt("c").unwrap(), Some("call".to_string()));
        assert_eq!(wire_right_opt("call").unwrap(), Some("call".to_string()));
        assert_eq!(wire_right_opt("CALL").unwrap(), Some("call".to_string()));
        assert_eq!(wire_right_opt("P").unwrap(), Some("put".to_string()));
        assert_eq!(wire_right_opt("put").unwrap(), Some("put".to_string()));
    }
}
