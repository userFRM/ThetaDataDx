//! Generated endpoint method bodies for [`MarketDataClient`].
//!
//! This module is the `include!` site for three build-time artifacts:
//!
//! - `mdds_list_endpoints_generated.rs` — simple list endpoints (returning
//!   `Vec<String>`) expanded through the [`list_endpoint!`] macro.
//! - `mdds_parsed_endpoints_generated.rs` — builder-style endpoints that
//!   parse a `DataTable` into a typed tick slice via [`parsed_endpoint!`].
//! - `mdds_streaming_endpoints_generated.rs` — streaming builders that pump
//!   a gRPC server-stream through a user callback.
//!
//! The generators live in
//! `build_support/endpoints/render/{mdds.rs, build_out.rs}`; the macro
//! definitions in [`crate::macros`]. Nothing in this module is hand-written.

use std::future::IntoFuture;
use std::pin::Pin;

use crate::columns::Ticks;
use crate::decode;
use crate::error::Error;
use crate::proto;

use crate::tdbe::types::tick::{
    CalendarDay, EodTick, GreeksAllTick, GreeksEodTick, GreeksFirstOrderTick,
    GreeksSecondOrderTick, GreeksThirdOrderTick, IndexPriceAtTimeTick, InterestRateTick, IvTick,
    MarketValueTick, OhlcTick, OpenInterestTick, OptionContract, PriceTick, QuoteTick,
    TradeGreeksAllTick, TradeGreeksFirstOrderTick, TradeGreeksImpliedVolatilityTick,
    TradeGreeksSecondOrderTick, TradeGreeksThirdOrderTick, TradeQuoteTick, TradeTick,
};

use super::client::MarketDataClient;
use super::wire_semantics::{
    normalize_date, normalize_expiration, wire_right_opt, wire_strike_opt,
};

/// Accepted symbol input for endpoints whose MDDS wire field is
/// `repeated string symbol`.
pub struct SymbolInput(Vec<String>);

impl SymbolInput {
    fn into_vec(self) -> Vec<String> {
        self.0
    }
}

impl From<&str> for SymbolInput {
    fn from(value: &str) -> Self {
        Self(vec![value.to_string()])
    }
}

impl From<String> for SymbolInput {
    fn from(value: String) -> Self {
        Self(vec![value])
    }
}

impl From<&[&str]> for SymbolInput {
    fn from(values: &[&str]) -> Self {
        Self(values.iter().map(|value| (*value).to_string()).collect())
    }
}

impl<const N: usize> From<&[&str; N]> for SymbolInput {
    fn from(values: &[&str; N]) -> Self {
        Self::from(values.as_slice())
    }
}

impl From<Vec<&str>> for SymbolInput {
    fn from(values: Vec<&str>) -> Self {
        Self(values.into_iter().map(str::to_string).collect())
    }
}

impl From<&Vec<&str>> for SymbolInput {
    fn from(values: &Vec<&str>) -> Self {
        Self::from(values.as_slice())
    }
}

impl From<Vec<String>> for SymbolInput {
    fn from(values: Vec<String>) -> Self {
        Self(values)
    }
}

impl From<&[String]> for SymbolInput {
    fn from(values: &[String]) -> Self {
        Self(values.to_vec())
    }
}

// ─── MDDS-scoped wire canonicalizers ────────────────────────────────────
//
// These helpers are only meaningful for MDDS request construction, so
// they live next to the generated request builders rather than in the
// cross-cutting `wire_semantics` module. The three functions imported
// from `wire_semantics` above are shared with build-time code via
// `#[path]` reuse and stay out of this file.

/// Convert `time_of_day` values into the canonical `HH:MM:SS.SSS` format.
///
/// ThetaData's v3 at-time endpoints expect a formatted ET wall-clock time
/// such as `"09:30:00.000"`. This helper also accepts a millisecond-of-day
/// string like `"34200000"` and normalizes either form to `HH:MM:SS.SSS`,
/// so both the formatted and millisecond inputs reach the server canonical.
///
/// Invalid or out-of-range values are passed through unchanged so the
/// server can return the canonical validation error.
fn normalize_time_of_day(time_of_day: &str) -> String {
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
/// `right` are optional at the wire level and use builder defaults that
/// preserve the server wildcard behavior via `wire_strike_opt` and
/// `wire_right_opt`.
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

// Shared build-time source of truth for non-streaming list endpoints.
include!(concat!(
    env!("OUT_DIR"),
    "/mdds_list_endpoints_generated.rs"
));

// ═══════════════════════════════════════════════════════════════════════
//  Builder-pattern endpoints — structs + IntoFuture at module scope
// ═══════════════════════════════════════════════════════════════════════

// Shared build-time source of truth for non-streaming builder endpoints.
include!(concat!(
    env!("OUT_DIR"),
    "/mdds_parsed_endpoints_generated.rs"
));

// Shared build-time source of truth for streaming builder endpoints.
include!(concat!(
    env!("OUT_DIR"),
    "/mdds_streaming_endpoints_generated.rs"
));

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
}
