//! Generated endpoint method bodies for [`MddsClient`].
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

use crate::decode;
use crate::error::Error;
use crate::proto;

use tdbe::types::tick::{
    CalendarDay, EodTick, GreeksTick, InterestRateTick, IvTick, MarketValueTick, OhlcTick,
    OpenInterestTick, OptionContract, PriceTick, QuoteTick, TradeQuoteTick, TradeTick,
};

use super::client::MddsClient;
use super::normalize::{
    contract_spec, normalize_expiration, normalize_interval, normalize_time_of_day, wire_right_opt,
    wire_strike_opt,
};
use super::validate::validate_date;

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
