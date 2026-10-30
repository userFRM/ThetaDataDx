// Layout guards for the hand-written `#[repr(C)]` ABI structs that are
// not emitted from a schema (so no generated assert covers them). The
// generated tick and streaming event structs are pinned in
// `tick_layout_asserts.hpp.inc` and `fpss_layout_asserts.hpp.inc`; the
// structs below are declared by hand on both sides, so these
// hand-written asserts are the only thing that fails the build if a
// field width, padding gap, or member order drifts from the Rust
// definitions in `thetadatadx-ffi/src` (`types.rs`, `streaming.rs`).
//
// Sizes and offsets are the LP64 layout (x86_64 / aarch64 Linux, macOS)
// of the matching `#[repr(C)]` structs.

// ── ThetaDataDxOptionContract (thetadatadx-ffi/src/types.rs) ──
// symbol (const char*) @0, expiration (int32_t) @8, 4-byte pad,
// strike (double) @16, right (uint32_t) @24, 4-byte tail pad → 32 bytes.
static_assert(offsetof(ThetaDataDxOptionContract, symbol) == 0,
              "ThetaDataDxOptionContract::symbol offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContract, expiration) == 8,
              "ThetaDataDxOptionContract::expiration offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContract, strike) == 16,
              "ThetaDataDxOptionContract::strike offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContract, right) == 24,
              "ThetaDataDxOptionContract::right offset drifted from Rust");
static_assert(sizeof(ThetaDataDxOptionContract) == 32,
              "ThetaDataDxOptionContract total size drifted from Rust");

// ── ThetaDataDxOptionContractArray (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxOptionContractArray, data) == 0,
              "ThetaDataDxOptionContractArray::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxOptionContractArray, len) == 8,
              "ThetaDataDxOptionContractArray::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxOptionContractArray) == 16,
              "ThetaDataDxOptionContractArray total size drifted from Rust");

// ── ThetaDataDxStringArray (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxStringArray, data) == 0,
              "ThetaDataDxStringArray::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxStringArray, len) == 8,
              "ThetaDataDxStringArray::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxStringArray) == 16,
              "ThetaDataDxStringArray total size drifted from Rust");

// ── ThetaDataDxColumnPresence (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxColumnPresence, names) == 0,
              "ThetaDataDxColumnPresence::names offset drifted from Rust");
static_assert(offsetof(ThetaDataDxColumnPresence, len) == 8,
              "ThetaDataDxColumnPresence::len offset drifted from Rust");
static_assert(offsetof(ThetaDataDxColumnPresence, symbol) == 16,
              "ThetaDataDxColumnPresence::symbol offset drifted from Rust");
static_assert(offsetof(ThetaDataDxColumnPresence, symbols) == 24,
              "ThetaDataDxColumnPresence::symbols offset drifted from Rust");
static_assert(offsetof(ThetaDataDxColumnPresence, symbols_len) == 32,
              "ThetaDataDxColumnPresence::symbols_len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxColumnPresence) == 40,
              "ThetaDataDxColumnPresence total size drifted from Rust");

// ── ThetaDataDxSubscription (thetadatadx-ffi/src/streaming.rs) ──
static_assert(offsetof(ThetaDataDxSubscription, kind) == 0,
              "ThetaDataDxSubscription::kind offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscription, contract) == 8,
              "ThetaDataDxSubscription::contract offset drifted from Rust");
static_assert(sizeof(ThetaDataDxSubscription) == 16,
              "ThetaDataDxSubscription total size drifted from Rust");

// ── ThetaDataDxSubscriptionArray (thetadatadx-ffi/src/streaming.rs) ──
static_assert(offsetof(ThetaDataDxSubscriptionArray, data) == 0,
              "ThetaDataDxSubscriptionArray::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionArray, len) == 8,
              "ThetaDataDxSubscriptionArray::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxSubscriptionArray) == 16,
              "ThetaDataDxSubscriptionArray total size drifted from Rust");

// ── ThetaDataDxSubscriptionRequest (thetadatadx-ffi/src/streaming.rs) ──
// scope (int32_t) @0, kind (int32_t) @4, then six pointer fields packed
// 8 bytes apart with no interior padding → 48 bytes.
static_assert(offsetof(ThetaDataDxSubscriptionRequest, scope) == 0,
              "ThetaDataDxSubscriptionRequest::scope offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, kind) == 4,
              "ThetaDataDxSubscriptionRequest::kind offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, symbol) == 8,
              "ThetaDataDxSubscriptionRequest::symbol offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, expiration) == 16,
              "ThetaDataDxSubscriptionRequest::expiration offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, strike) == 24,
              "ThetaDataDxSubscriptionRequest::strike offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, right) == 32,
              "ThetaDataDxSubscriptionRequest::right offset drifted from Rust");
static_assert(offsetof(ThetaDataDxSubscriptionRequest, sec_type) == 40,
              "ThetaDataDxSubscriptionRequest::sec_type offset drifted from Rust");
static_assert(sizeof(ThetaDataDxSubscriptionRequest) == 48,
              "ThetaDataDxSubscriptionRequest total size drifted from Rust");

// ── ThetaDataDxArrowBytes (thetadatadx-ffi/src/types.rs) ──
static_assert(offsetof(ThetaDataDxArrowBytes, data) == 0,
              "ThetaDataDxArrowBytes::data offset drifted from Rust");
static_assert(offsetof(ThetaDataDxArrowBytes, len) == 8,
              "ThetaDataDxArrowBytes::len offset drifted from Rust");
static_assert(sizeof(ThetaDataDxArrowBytes) == 16,
              "ThetaDataDxArrowBytes total size drifted from Rust");

// ── ThetaDataDxEndpointRequestOptions ──
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, venue) == 0,
              "ThetaDataDxEndpointRequestOptions::venue offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, min_time) == 8,
              "ThetaDataDxEndpointRequestOptions::min_time offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, current_day_only) == 16,
              "ThetaDataDxEndpointRequestOptions::current_day_only offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_current_day_only) == 20,
              "ThetaDataDxEndpointRequestOptions::has_current_day_only offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, date) == 24,
              "ThetaDataDxEndpointRequestOptions::date offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, interval) == 32,
              "ThetaDataDxEndpointRequestOptions::interval offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, start_time) == 40,
              "ThetaDataDxEndpointRequestOptions::start_time offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, end_time) == 48,
              "ThetaDataDxEndpointRequestOptions::end_time offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, start_date) == 56,
              "ThetaDataDxEndpointRequestOptions::start_date offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, end_date) == 64,
              "ThetaDataDxEndpointRequestOptions::end_date offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, exclusive) == 72,
              "ThetaDataDxEndpointRequestOptions::exclusive offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_exclusive) == 76,
              "ThetaDataDxEndpointRequestOptions::has_exclusive offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, use_time_based_trade_quote) == 80,
              "ThetaDataDxEndpointRequestOptions::use_time_based_trade_quote offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_use_time_based_trade_quote) == 84,
              "ThetaDataDxEndpointRequestOptions::has_use_time_based_trade_quote offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, strike) == 88,
              "ThetaDataDxEndpointRequestOptions::strike offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, right) == 96,
              "ThetaDataDxEndpointRequestOptions::right offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, symbol) == 104,
              "ThetaDataDxEndpointRequestOptions::symbol offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, max_dte) == 112,
              "ThetaDataDxEndpointRequestOptions::max_dte offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_max_dte) == 116,
              "ThetaDataDxEndpointRequestOptions::has_max_dte offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, strike_range) == 120,
              "ThetaDataDxEndpointRequestOptions::strike_range offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_strike_range) == 124,
              "ThetaDataDxEndpointRequestOptions::has_strike_range offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, annual_dividend) == 128,
              "ThetaDataDxEndpointRequestOptions::annual_dividend offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_annual_dividend) == 136,
              "ThetaDataDxEndpointRequestOptions::has_annual_dividend offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, annual_dividend_amount) == 144,
              "ThetaDataDxEndpointRequestOptions::annual_dividend_amount offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_annual_dividend_amount) == 152,
              "ThetaDataDxEndpointRequestOptions::has_annual_dividend_amount offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, rate_type) == 160,
              "ThetaDataDxEndpointRequestOptions::rate_type offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, rate_value) == 168,
              "ThetaDataDxEndpointRequestOptions::rate_value offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_rate_value) == 176,
              "ThetaDataDxEndpointRequestOptions::has_rate_value offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, stock_price) == 184,
              "ThetaDataDxEndpointRequestOptions::stock_price offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_stock_price) == 192,
              "ThetaDataDxEndpointRequestOptions::has_stock_price offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, version) == 200,
              "ThetaDataDxEndpointRequestOptions::version offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, use_market_value) == 208,
              "ThetaDataDxEndpointRequestOptions::use_market_value offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_use_market_value) == 212,
              "ThetaDataDxEndpointRequestOptions::has_use_market_value offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, binomial_steps) == 216,
              "ThetaDataDxEndpointRequestOptions::binomial_steps offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_binomial_steps) == 220,
              "ThetaDataDxEndpointRequestOptions::has_binomial_steps offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, underlyer_use_nbbo) == 224,
              "ThetaDataDxEndpointRequestOptions::underlyer_use_nbbo offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_underlyer_use_nbbo) == 228,
              "ThetaDataDxEndpointRequestOptions::has_underlyer_use_nbbo offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, option_use_nbbo) == 232,
              "ThetaDataDxEndpointRequestOptions::option_use_nbbo offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_option_use_nbbo) == 236,
              "ThetaDataDxEndpointRequestOptions::has_option_use_nbbo offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, perf_boost_intraday) == 240,
              "ThetaDataDxEndpointRequestOptions::perf_boost_intraday offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_perf_boost_intraday) == 244,
              "ThetaDataDxEndpointRequestOptions::has_perf_boost_intraday offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, timeout_ms) == 248,
              "ThetaDataDxEndpointRequestOptions::timeout_ms offset drifted from Rust");
static_assert(offsetof(ThetaDataDxEndpointRequestOptions, has_timeout_ms) == 256,
              "ThetaDataDxEndpointRequestOptions::has_timeout_ms offset drifted from Rust");
static_assert(sizeof(ThetaDataDxEndpointRequestOptions) == 264,
              "ThetaDataDxEndpointRequestOptions total size drifted from Rust");
