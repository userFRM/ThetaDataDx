//! Per-language emitters for the checked-in SDK projections.
//!
//! Each submodule owns one render target (Python, TypeScript, C++, FFI,
//! per-language live-validators, enums). The build script never compiles
//! this tree — only the `generate_sdk_surfaces` binary reaches here.

mod config_accessors;
mod cpp;
mod cpp_validate;
mod doc;
mod enums;
mod ffi;
mod python;
mod python_stub;
mod python_validate;
mod sdk_files;
mod typescript;

pub(super) use sdk_files::{check_sdk_generated_files, write_sdk_generated_files};

/// Whether a streamed endpoint's `.stream` output can fan out across
/// concurrent sub-requests under `bulk_fetch = "auto"`. Only intraday
/// tick / bar / greeks history endpoints shard; snapshots, lists, at-time
/// queries, and the daily-only EOD / open-interest families run
/// single-stream, so their stream docs must not claim a fan-out.
///
/// Mirrors the shardable set in `crate::mdds::shard::descriptor` (the
/// runtime source of truth) — keep the two lists in sync.
fn endpoint_can_fan_out(name: &str) -> bool {
    matches!(
        name,
        "option_history_trade"
            | "option_history_ohlc"
            | "option_history_trade_greeks_all"
            | "option_history_trade_greeks_first_order"
            | "option_history_trade_greeks_second_order"
            | "option_history_trade_greeks_third_order"
            | "option_history_trade_greeks_implied_volatility"
            | "option_history_trade_quote"
            | "option_history_quote"
            | "option_history_greeks_all"
            | "option_history_greeks_first_order"
            | "option_history_greeks_second_order"
            | "option_history_greeks_third_order"
            | "option_history_greeks_implied_volatility"
            | "stock_history_trade"
            | "stock_history_trade_quote"
            | "stock_history_ohlc"
            | "stock_history_quote"
            | "index_history_price"
            | "index_history_ohlc"
    )
}
