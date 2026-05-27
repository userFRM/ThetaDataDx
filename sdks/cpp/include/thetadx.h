/**
 * thetadatadx C FFI header.
 *
 * This header declares the C interface to the thetadatadx Rust SDK.
 * Used by both the C++ wrapper and any other C-compatible language.
 *
 * Memory model:
 * - Opaque handles (TdxCredentials*, TdxClient*, TdxConfig*) are heap-allocated
 *   by the Rust side and MUST be freed with the corresponding tdx_*_free function.
 * - Tick data is returned as #[repr(C)] struct arrays. Each array type has a
 *   corresponding tdx_*_array_free function that MUST be called.
 * - String arrays (TdxStringArray) must be freed with tdx_string_array_free.
 * - Functions that can fail return empty arrays (data=NULL, len=0) and set a
 *   thread-local error string retrievable via tdx_last_error().
 */

#ifndef THETADX_H
#define THETADX_H

#include <stdint.h>
#include <stddef.h>
#ifndef __cplusplus
#include <stdbool.h>
#endif

#if defined(_MSC_VER)
#define TDX_ALIGN64_BEGIN __declspec(align(64))
#define TDX_ALIGN64_END
#else
#define TDX_ALIGN64_BEGIN
#define TDX_ALIGN64_END __attribute__((aligned(64)))
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ── Opaque handle types ── */
typedef struct TdxCredentials TdxCredentials;
typedef struct TdxClient TdxClient;
typedef struct TdxConfig TdxConfig;
typedef struct TdxFpssHandle TdxFpssHandle;
typedef struct TdxUnified TdxUnified;

/* Generated request-options bridge shared with Rust FFI. */
#include "endpoint_request_options.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  #[repr(C)] tick types — layout-compatible with Rust tdbe structs      */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All tick structs are 64-byte aligned to match Rust's #[repr(C, align(64))].
 * Explicit tail padding is part of that ABI contract so C/C++ array stepping
 * stays byte-for-byte compatible with Rust. Price fields are f64 (double). */

TDX_ALIGN64_BEGIN typedef struct {
    int32_t date;
    int32_t is_open;
    int32_t open_time;
    int32_t close_time;
    int32_t status;
    uint8_t _tail_padding[44];
} TdxCalendarDay TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t ms_of_day2;
    double open;
    double high;
    double low;
    double close;
    /* volume/count are int64 to match the core crate and prevent
     * overflow on high-volume symbols (2.1B+ cumulative volume). */
    int64_t volume;
    int64_t count;
    int32_t bid_size;
    int32_t bid_exchange;
    double bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    /* 4 bytes padding before f64 */
    double ask;
    int32_t ask_condition;
    int32_t date;
    int32_t expiration;
    /* 4 bytes padding before f64 */
    double strike;
    int32_t right;
    uint8_t _tail_padding[4];
} TdxEodTick TDX_ALIGN64_END;

/* Full-union Greeks tick (option_*_greeks_all, interval-sampled). */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double bid;
    double ask;
    double implied_volatility;
    double delta;
    double gamma;
    double theta;
    double vega;
    double rho;
    double iv_error;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double speed;
    double zomma;
    double color;
    double ultima;
    double d1;
    double d2;
    double dual_delta;
    double dual_gamma;
    double epsilon;
    double lambda;
    double vera;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[20];
} TdxGreeksAllTick TDX_ALIGN64_END;

/* End-of-day Greeks tick (option_history_greeks_eod) -- fuses every
 * Greek with the twelve EOD trade/quote columns (open/high/low/close,
 * volume, count, bid_size, bid_exchange, bid_condition, ask_size,
 * ask_exchange, ask_condition) absent from the interval-sampled
 * GreeksAllTick. */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double open;
    double high;
    double low;
    double close;
    int64_t volume;
    int64_t count;
    int32_t bid_size;
    int32_t bid_exchange;
    double bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    double ask;
    int32_t ask_condition;
    /* 4 bytes padding before f64 */
    double delta;
    double theta;
    double vega;
    double rho;
    double epsilon;
    double lambda;
    double gamma;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double vera;
    double speed;
    double zomma;
    double color;
    double ultima;
    double d1;
    double d2;
    double dual_delta;
    double dual_gamma;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[4];
} TdxGreeksEodTick TDX_ALIGN64_END;

/* First-order Greeks subset tick (option_*_greeks_first_order). */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double bid;
    double ask;
    double delta;
    double theta;
    double vega;
    double rho;
    double epsilon;
    double lambda;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[4];
} TdxGreeksFirstOrderTick TDX_ALIGN64_END;

/* Second-order Greeks subset tick (option_*_greeks_second_order). */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double bid;
    double ask;
    double gamma;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[12];
} TdxGreeksSecondOrderTick TDX_ALIGN64_END;

/* Third-order Greeks subset tick (option_*_greeks_third_order). The
 * vendor's third-order schema does not publish `vera`. */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double bid;
    double ask;
    double speed;
    double zomma;
    double color;
    double ultima;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[20];
} TdxGreeksThirdOrderTick TDX_ALIGN64_END;

/* Per-OPRA-trade union Greeks tick (option_history_trade_greeks_all).
 * Carries the nine trade-side execution columns alongside every Greek
 * the server publishes -- distinct from the interval-sampled
 * TdxGreeksAllTick whose rows carry the bid/ask quote pair instead. */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    double delta;
    double theta;
    double vega;
    double rho;
    double epsilon;
    double lambda;
    double gamma;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double vera;
    double speed;
    double zomma;
    double color;
    double ultima;
    double d1;
    double d2;
    double dual_delta;
    double dual_gamma;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[60];
} TdxTradeGreeksAllTick TDX_ALIGN64_END;

/* Per-OPRA-trade first-order Greeks tick
 * (option_history_trade_greeks_first_order). */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    double delta;
    double theta;
    double vega;
    double rho;
    double epsilon;
    double lambda;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[44];
} TdxTradeGreeksFirstOrderTick TDX_ALIGN64_END;

/* Per-OPRA-trade second-order Greeks tick
 * (option_history_trade_greeks_second_order). */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    double gamma;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[52];
} TdxTradeGreeksSecondOrderTick TDX_ALIGN64_END;

/* Per-OPRA-trade third-order Greeks tick
 * (option_history_trade_greeks_third_order). The vendor's third-order
 * schema does not publish `vera`. */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    double speed;
    double zomma;
    double color;
    double ultima;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[60];
} TdxTradeGreeksThirdOrderTick TDX_ALIGN64_END;

/* Per-OPRA-trade implied-volatility tick
 * (option_history_trade_greeks_implied_volatility). Carries only the
 * single `implied_volatility` + `iv_error` pair (NOT the bid/mid/ask IV
 * triple of the interval-sampled TdxIvTick). */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[28];
} TdxTradeGreeksImpliedVolatilityTick TDX_ALIGN64_END;

/* InterestRateTick (2 fields). End-of-day interest rate (percent).
 * Wire shape per docs.thetadata.us/operations/interest_rate_history_eod.html
 * verified against terminal jar build 202605221:
 *   date  <- Text "YYYY-MM-DD" header `created`, parsed to YYYYMMDD i32
 *   rate  <- Number percent (e.g. 4.36 for SOFR 2025-04-28)
 */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t date;
    /* 4 bytes padding before f64 */
    double rate;
    uint8_t _tail_padding[48];
} TdxInterestRateTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double bid;
    double bid_implied_volatility;
    double midpoint;
    double implied_volatility;
    double ask;
    double ask_implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before f64 */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[28];
} TdxIvTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double market_bid;
    double market_ask;
    double market_price;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[8];
} TdxMarketValueTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double open;
    double high;
    double low;
    double close;
    /* volume/count are int64 to match the core crate and prevent
     * overflow on high-volume symbols (2.1B+ cumulative volume). */
    int64_t volume;
    int64_t count;
    /* SIP-rule VWAP for the bar. Snapshot endpoints leave this as 0.0
     * via the optional-column path. */
    double vwap;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[44];
} TdxOhlcTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t open_interest;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[32];
} TdxOpenInterestTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double price;
    int32_t date;
    uint8_t _tail_padding[40];
} TdxPriceTick TDX_ALIGN64_END;

/* Trade-shaped index price tick (index_at_time_price) -- carries the
 * seven trade-side execution columns (sequence, ext_condition1..4,
 * condition, size, exchange) the bare TdxPriceTick silently dropped,
 * including the SIP-exchange attribution field. */
TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    int32_t date;
    uint8_t _tail_padding[12];
} TdxIndexPriceAtTimeTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    /* 4 bytes padding before f64 */
    double bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    /* 4 bytes padding before f64 */
    double ask;
    int32_t ask_condition;
    int32_t date;
    int32_t expiration;
    /* 4 bytes padding before f64 */
    double strike;
    int32_t right;
    /* 4 bytes padding before f64 */
    double midpoint;
    uint8_t _tail_padding[40];
} TdxQuoteTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    int32_t condition_flags;
    int32_t price_flags;
    int32_t volume_type;
    int32_t records_back;
    int32_t quote_ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    /* 4 bytes padding before f64 */
    double bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    /* 4 bytes padding before f64 */
    double ask;
    int32_t ask_condition;
    int32_t date;
    int32_t expiration;
    /* 4 bytes padding before f64 */
    double strike;
    int32_t right;
    uint8_t _tail_padding[48];
} TdxTradeQuoteTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before f64 */
    double price;
    int32_t condition_flags;
    int32_t price_flags;
    int32_t volume_type;
    int32_t records_back;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[40];
} TdxTradeTick TDX_ALIGN64_END;

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Typed array return types                                              */
/* ═══════════════════════════════════════════════════════════════════════ */

typedef struct { const TdxEodTick* data; size_t len; } TdxEodTickArray;
typedef struct { const TdxOhlcTick* data; size_t len; } TdxOhlcTickArray;
typedef struct { const TdxTradeTick* data; size_t len; } TdxTradeTickArray;
typedef struct { const TdxQuoteTick* data; size_t len; } TdxQuoteTickArray;
typedef struct { const TdxGreeksAllTick* data; size_t len; } TdxGreeksAllTickArray;
typedef struct { const TdxGreeksEodTick* data; size_t len; } TdxGreeksEodTickArray;
typedef struct { const TdxGreeksFirstOrderTick* data; size_t len; } TdxGreeksFirstOrderTickArray;
typedef struct { const TdxGreeksSecondOrderTick* data; size_t len; } TdxGreeksSecondOrderTickArray;
typedef struct { const TdxGreeksThirdOrderTick* data; size_t len; } TdxGreeksThirdOrderTickArray;
typedef struct { const TdxTradeGreeksAllTick* data; size_t len; } TdxTradeGreeksAllTickArray;
typedef struct { const TdxTradeGreeksFirstOrderTick* data; size_t len; } TdxTradeGreeksFirstOrderTickArray;
typedef struct { const TdxTradeGreeksSecondOrderTick* data; size_t len; } TdxTradeGreeksSecondOrderTickArray;
typedef struct { const TdxTradeGreeksThirdOrderTick* data; size_t len; } TdxTradeGreeksThirdOrderTickArray;
typedef struct { const TdxTradeGreeksImpliedVolatilityTick* data; size_t len; } TdxTradeGreeksImpliedVolatilityTickArray;
typedef struct { const TdxIvTick* data; size_t len; } TdxIvTickArray;
typedef struct { const TdxPriceTick* data; size_t len; } TdxPriceTickArray;
typedef struct { const TdxIndexPriceAtTimeTick* data; size_t len; } TdxIndexPriceAtTimeTickArray;
typedef struct { const TdxOpenInterestTick* data; size_t len; } TdxOpenInterestTickArray;
typedef struct { const TdxMarketValueTick* data; size_t len; } TdxMarketValueTickArray;
typedef struct { const TdxCalendarDay* data; size_t len; } TdxCalendarDayArray;
typedef struct { const TdxInterestRateTick* data; size_t len; } TdxInterestRateTickArray;
typedef struct { const TdxTradeQuoteTick* data; size_t len; } TdxTradeQuoteTickArray;

/* ── OptionContract (has heap-allocated symbol string) ── */

typedef struct {
    const char* symbol;     /* heap-allocated, freed with tdx_option_contract_array_free */
    int32_t expiration;
    /* 4 bytes padding before f64 */
    double strike;
    int32_t right;
} TdxOptionContract;

typedef struct { const TdxOptionContract* data; size_t len; } TdxOptionContractArray;

/* ── String array (for list endpoints) ── */

typedef struct {
    const char* const* data;  /* array of NUL-terminated C strings */
    size_t len;
} TdxStringArray;

/* ── Greeks result (standalone tdx_all_greeks) ── */

typedef struct {
    double value;
    double delta;
    double gamma;
    double theta;
    double vega;
    double rho;
    double epsilon;
    double lambda;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double vera;
    double speed;
    double zomma;
    double color;
    double ultima;
    double iv;
    double iv_error;
    double d1;
    double d2;
    double dual_delta;
    double dual_gamma;
} TdxGreeksResult;

/* ── Subscription types (active_subscriptions) ── */

typedef struct {
    const char* kind;      /* "Quote", "Trade", or "OpenInterest" */
    const char* contract;  /* "SPY" or "SPY 20260417 550 C" */
} TdxSubscription;

typedef struct {
    const TdxSubscription* data;
    size_t len;
} TdxSubscriptionArray;

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Free functions for typed arrays                                       */
/* ═══════════════════════════════════════════════════════════════════════ */

void tdx_eod_tick_array_free(TdxEodTickArray arr);
void tdx_ohlc_tick_array_free(TdxOhlcTickArray arr);
void tdx_trade_tick_array_free(TdxTradeTickArray arr);
void tdx_quote_tick_array_free(TdxQuoteTickArray arr);
void tdx_greeks_all_tick_array_free(TdxGreeksAllTickArray arr);
void tdx_greeks_eod_tick_array_free(TdxGreeksEodTickArray arr);
void tdx_greeks_first_order_tick_array_free(TdxGreeksFirstOrderTickArray arr);
void tdx_greeks_second_order_tick_array_free(TdxGreeksSecondOrderTickArray arr);
void tdx_greeks_third_order_tick_array_free(TdxGreeksThirdOrderTickArray arr);
void tdx_trade_greeks_all_tick_array_free(TdxTradeGreeksAllTickArray arr);
void tdx_trade_greeks_first_order_tick_array_free(TdxTradeGreeksFirstOrderTickArray arr);
void tdx_trade_greeks_second_order_tick_array_free(TdxTradeGreeksSecondOrderTickArray arr);
void tdx_trade_greeks_third_order_tick_array_free(TdxTradeGreeksThirdOrderTickArray arr);
void tdx_trade_greeks_implied_volatility_tick_array_free(TdxTradeGreeksImpliedVolatilityTickArray arr);
void tdx_iv_tick_array_free(TdxIvTickArray arr);
void tdx_price_tick_array_free(TdxPriceTickArray arr);
void tdx_index_price_at_time_tick_array_free(TdxIndexPriceAtTimeTickArray arr);
void tdx_open_interest_tick_array_free(TdxOpenInterestTickArray arr);
void tdx_market_value_tick_array_free(TdxMarketValueTickArray arr);
void tdx_calendar_day_array_free(TdxCalendarDayArray arr);
void tdx_interest_rate_tick_array_free(TdxInterestRateTickArray arr);
void tdx_trade_quote_tick_array_free(TdxTradeQuoteTickArray arr);
void tdx_option_contract_array_free(TdxOptionContractArray arr);
void tdx_string_array_free(TdxStringArray arr);
void tdx_greeks_result_free(TdxGreeksResult* result);
void tdx_subscription_array_free(TdxSubscriptionArray* arr);

/* ── Error ── */

/** Retrieve the last error message (or NULL if no error).
 *  The returned pointer is valid until the next FFI call on the same thread.
 *  Do NOT free this pointer. */
const char* tdx_last_error(void);

/** Clear the thread-local error string.
 *  Higher-level wrappers should call this before issuing an FFI call so
 *  they can distinguish "the call set a new error" from "the previous
 *  call left a stale error in the slot" when an empty value (e.g. zero
 *  rows) is also a valid success outcome. */
void tdx_clear_error(void);

/** Typed discriminant of the last FFI error on this thread. Higher-
 *  level bindings (the C++ exception hierarchy below, the typed napi
 *  error subclasses in the TypeScript SDK) dispatch on this to pick
 *  the right exception / error subclass without substring-matching
 *  the formatted error string. Codes match the constants below; the
 *  string from `tdx_last_error()` carries the diagnostic.
 *
 *  Returns `TDX_ERR_NONE` when no error is set or after
 *  `tdx_clear_error()`. */
int32_t tdx_last_error_code(void);

/* Error-code discriminants returned by `tdx_last_error_code()`.
 * Kept in sync with the `TDX_ERR_*` constants in `ffi/src/error.rs`. */
#define TDX_ERR_NONE 0
#define TDX_ERR_OTHER 1
#define TDX_ERR_AUTHENTICATION 2
#define TDX_ERR_INVALID_CREDENTIALS 3
#define TDX_ERR_SUBSCRIPTION 4
#define TDX_ERR_RATE_LIMIT 5
#define TDX_ERR_NOT_FOUND 6
#define TDX_ERR_DEADLINE_EXCEEDED 7
#define TDX_ERR_UNAVAILABLE 8
#define TDX_ERR_NETWORK 9
#define TDX_ERR_SCHEMA_MISMATCH 10
#define TDX_ERR_STREAM 11
#define TDX_ERR_CONFIG 12

/* ── Credentials ── */

/** Create credentials from email and password. Returns NULL on error. */
TdxCredentials* tdx_credentials_new(const char* email, const char* password);

/** Load credentials from a file (line 1 = email, line 2 = password). Returns NULL on error. */
TdxCredentials* tdx_credentials_from_file(const char* path);

/** Free a credentials handle. */
void tdx_credentials_free(TdxCredentials* creds);

/* ── Config ── */

/** Create a production config (ThetaData NJ datacenter). */
TdxConfig* tdx_config_production(void);

/** Create a dev streaming config (port 20200, infinite historical replay). */
TdxConfig* tdx_config_dev(void);

/** Create a stage streaming config (port 20100, testing, unstable). */
TdxConfig* tdx_config_stage(void);

/** Free a config handle. */
void tdx_config_free(TdxConfig* config);

/**
 * Set the streaming reconnect policy on a config handle.
 *   policy=0: Auto (default) -- auto-reconnect with split per-class attempt
 *             budgets. Generic transient failures (TimedOut, ServerRestarting,
 *             Unspecified) use the budget set by
 *             `tdx_config_set_reconnect_max_attempts`; the rate-limited
 *             (`TooManyRequests`) class uses
 *             `tdx_config_set_reconnect_max_rate_limited_attempts`. Counters
 *             reset after a continuous data-flow window configured via
 *             `tdx_config_set_reconnect_stable_window_secs`.
 *   policy=1: Manual -- no auto-reconnect.
 */
void tdx_config_set_reconnect_policy(TdxConfig* config, int policy);

/**
 * Set the per-class transient-failure attempt budget for the
 * auto-reconnect path. Default 3. No effect unless the reconnect
 * policy is Auto.
 */
void tdx_config_set_reconnect_max_attempts(TdxConfig* config,
                                           uint32_t max_attempts);

/**
 * Set the per-class rate-limited (`TooManyRequests`) attempt budget for
 * the auto-reconnect path. Default 100. No effect unless the reconnect
 * policy is Auto.
 */
void tdx_config_set_reconnect_max_rate_limited_attempts(
    TdxConfig* config, uint32_t max_rate_limited_attempts);

/**
 * Set the continuous successful-data-flow window (in seconds) after
 * which the auto-reconnect attempt counters reset. Default 60. No
 * effect unless the reconnect policy is Auto.
 */
void tdx_config_set_reconnect_stable_window_secs(TdxConfig* config,
                                                 uint64_t secs);

/**
 * Set the reconnect delay (ms) honoured for generic transient
 * disconnects (TimedOut, ServerRestarting, Unspecified, ...). Plumbed
 * through to the streaming I/O loop at connect time. Default 2_000.
 */
void tdx_config_set_reconnect_wait_ms(TdxConfig* config, uint64_t ms);

/**
 * Read the current reconnect wait_ms setting. Writes the configured
 * millisecond delay into *out_ms. Returns 0 on success, -1 if either
 * pointer is null.
 */
int32_t tdx_config_get_reconnect_wait_ms(const TdxConfig* config, uint64_t* out_ms);

/**
 * Set the reconnect delay (ms) honoured for `TooManyRequests`
 * rate-limited disconnects. Default 130_000 (matches the Java
 * terminal's 130 s rate-limit cooldown).
 */
void tdx_config_set_reconnect_wait_rate_limited_ms(TdxConfig* config, uint64_t ms);

/**
 * Read the current reconnect wait_rate_limited_ms setting. Same shape
 * as tdx_config_get_reconnect_wait_ms.
 */
int32_t tdx_config_get_reconnect_wait_rate_limited_ms(const TdxConfig* config, uint64_t* out_ms);

/**
 * Set the RuntimeConfig.tokio_worker_threads knob for embedded
 * runtimes built via RuntimeConfig::build_runtime.
 *
 *   has_value=false: encodes the auto-size sentinel (`None`); `n`
 *     is ignored. Defers to tokio's default sizing.
 *   has_value=true: encodes `Some(n)`. RuntimeConfig::build_runtime
 *     clamps `n=0` to `1`, but the explicit `Some(0)` is preserved
 *     across the C boundary matching the decode_threads shape so
 *     Python / TS / C++ bindings agree.
 *
 * Returns 0 on success, -1 if `config` is NULL.
 */
int32_t tdx_config_set_tokio_worker_threads_explicit(TdxConfig* config, bool has_value, size_t n);

/**
 * Read the current RuntimeConfig.tokio_worker_threads setting. Same
 * (has_value, n) shape as tdx_config_get_decode_threads:
 *
 *   *out_has_value=false: config holds `None` (auto-size). *out_n=0.
 *   *out_has_value=true:  config holds `Some(*out_n)`.
 *
 * Returns 0 on success, -1 if any pointer is null.
 */
int32_t tdx_config_get_tokio_worker_threads(const TdxConfig* config, bool* out_has_value, size_t* out_n);

/* ── RetryPolicy field setters/getters ── */

/**
 * Set the initial backoff delay (ms) for the historical-channel retry policy.
 * Default 250. Subsequent retries double from here, capped at
 * tdx_config_set_retry_max_delay_ms.
 */
void tdx_config_set_retry_initial_delay_ms(TdxConfig* config, uint64_t ms);

/** Read retry.initial_delay (ms). */
int32_t tdx_config_get_retry_initial_delay_ms(const TdxConfig* config, uint64_t* out_ms);

/**
 * Set the upper-bound backoff delay (ms) for the historical-channel retry policy.
 * Default 30_000 (30 s).
 */
void tdx_config_set_retry_max_delay_ms(TdxConfig* config, uint64_t ms);

/** Read retry.max_delay (ms). */
int32_t tdx_config_get_retry_max_delay_ms(const TdxConfig* config, uint64_t* out_ms);

/**
 * Set the total attempt budget for the historical-channel retry policy. 1 disables
 * retry (single call only); higher values permit retries up to
 * max_attempts - 1 after the initial call. Default 5.
 */
void tdx_config_set_retry_max_attempts(TdxConfig* config, uint32_t n);

/** Read retry.max_attempts. */
int32_t tdx_config_get_retry_max_attempts(const TdxConfig* config, uint32_t* out_n);

/**
 * Toggle AWS-style full-jitter on the historical-channel retry policy. Default
 * true. false gives the deterministic backoff schedule
 * min(max_delay, initial * 2^attempt), useful for tests.
 */
void tdx_config_set_retry_jitter(TdxConfig* config, bool jitter);

/** Read retry.jitter. */
int32_t tdx_config_get_retry_jitter(const TdxConfig* config, bool* out_jitter);

/* ── FlatFilesConfig field setters/getters ── */

/**
 * Set the total attempt budget for the flatfile driver retry loop.
 * 1 disables retry (single call only); higher values permit retries
 * up to max_attempts - 1 after the initial call. Default 3.
 * Validated to the range [1, 10] at connect time.
 */
void tdx_config_set_flatfiles_max_attempts(TdxConfig* config, uint32_t n);

/** Read flatfiles.max_attempts. */
int32_t tdx_config_get_flatfiles_max_attempts(const TdxConfig* config, uint32_t* out_n);

/**
 * Set the initial backoff delay (seconds) for the flatfile driver
 * retry loop. Doubles per attempt up to max_backoff_secs. Default 1.
 */
void tdx_config_set_flatfiles_initial_backoff_secs(TdxConfig* config, uint64_t secs);

/** Read flatfiles.initial_backoff (seconds). */
int32_t tdx_config_get_flatfiles_initial_backoff_secs(const TdxConfig* config, uint64_t* out_secs);

/**
 * Set the upper-bound backoff delay (seconds) for the flatfile
 * driver retry loop. The doubling schedule never exceeds this value
 * regardless of attempt number. Default 4. Must be >= initial_backoff
 * (rejected at connect-time validate otherwise).
 */
void tdx_config_set_flatfiles_max_backoff_secs(TdxConfig* config, uint64_t secs);

/** Read flatfiles.max_backoff (seconds). */
int32_t tdx_config_get_flatfiles_max_backoff_secs(const TdxConfig* config, uint64_t* out_secs);

/**
 * Set streaming flush mode on a config handle.
 *   mode=0: Batched (default) -- flush only on PING every 100ms.
 *   mode=1: Immediate -- flush after every frame write.
 */
void tdx_config_set_flush_mode(TdxConfig* config, int mode);

/**
 * Set streaming OHLCVC derivation on a config handle.
 *   enabled=1 (default): derive OHLCVC bars locally from trade events.
 *   enabled=0: only emit server-sent OHLCVC frames (lower overhead).
 */
void tdx_config_set_derive_ohlcvc(TdxConfig* config, int enabled);

/* ── Decode pool sizing ── */

/**
 * Set the number of concurrent in-flight gRPC requests.
 *
 *   n=0 (default): auto-detect from the Nexus subscription tier
 *     (Free=1 / Value=2 / Standard=4 / Pro=8).
 *   n>0: explicit cap, clamped to the tier cap at connect time
 *     with a `tracing::warn!` if exceeded.
 */
void tdx_config_set_concurrent_requests(TdxConfig* config, uint32_t n);

/**
 * Set the warn_on_buffered_threshold_bytes ceiling on a config.
 *
 * Pre-stream-API endpoints (the non-`.stream()` shape) log a
 * `tracing::warn!` when a buffered response's decoded total exceeds
 * this threshold, guiding users to the streaming variant. The
 * payload is still delivered.
 *
 *   n=0 disables the warning entirely.
 *   default = 100 * 1024 * 1024 (100 MiB).
 */
void tdx_config_set_warn_on_buffered_threshold_bytes(TdxConfig* config, size_t n);

/**
 * Read the current warn_on_buffered_threshold_bytes setting.
 *
 * Writes the configured byte count into *out_n. Returns 0 on success,
 * -1 if either pointer is null.
 */
int32_t tdx_config_get_warn_on_buffered_threshold_bytes(const TdxConfig* config, size_t* out_n);

/**
 * Set the per-thread decoder ring size.
 *
 * Must be a power of two, >= 64. Invalid values are rejected at the
 * setter: the config is left unchanged and the failure reason is
 * written to thread-local storage retrievable via tdx_last_error().
 * Default is 256.
 */
void tdx_config_set_decoder_ring_size(TdxConfig* config, uint32_t n);

/* ── Decode pipeline ── */

/**
 * Set the stage-2 worker thread count for the two-stage decode
 * pipeline.
 *
 * Stage-2 runs prost decode + Tick build off a bounded MPSC queue
 * fed by the stage-1 per-channel decompress threads.
 *
 *   has_value=false: encodes the auto-size sentinel (`None`); `n`
 *     is ignored. Pool sizes from available_parallelism() at
 *     connect time.
 *   has_value=true: encodes `Some(n)`. The pool clamps internally
 *     to a minimum of 1; explicit 0 clamps but is preserved as
 *     `Some(0)` so Python / TS / C++ bindings agree on the shape.
 *
 * Returns 0 on success, -1 if `config` is NULL.
 */
int32_t tdx_config_set_decode_threads_explicit(TdxConfig* config, bool has_value, size_t n);

/**
 * Set the bounded queue depth between stage-1 and stage-2 of the
 * two-stage decode pipeline.
 *
 * When stage-2 cannot keep up, stage-1 parks rather than drops --
 * silent drops on a market-data feed are unacceptable.
 *
 *   has_value=false: encodes the auto-size sentinel (`None`); `n`
 *     is ignored. Queue sizes to `concurrent_requests * 64` (floor
 *     of 64) at connect time.
 *   has_value=true: encodes `Some(n)`. The queue clamps internally
 *     to a minimum of 1.
 *
 * Returns 0 on success, -1 if `config` is NULL.
 */
int32_t tdx_config_set_decode_queue_depth_explicit(TdxConfig* config, bool has_value, size_t n);

/**
 * Read the current decode_threads setting.
 *
 * On return:
 *   *out_has_value=0: config holds None (auto-size); *out_n=0.
 *   *out_has_value=1: config holds Some(*out_n).
 *
 * Returns 0 on success, -1 if any pointer is NULL.
 */
int32_t tdx_config_get_decode_threads(const TdxConfig* config, bool* out_has_value, size_t* out_n);

/**
 * Read the current decode_queue_depth setting. Same semantics as
 * tdx_config_get_decode_threads.
 */
int32_t tdx_config_get_decode_queue_depth(const TdxConfig* config, bool* out_has_value, size_t* out_n);

/**
 * Legacy n-only setter for decode_threads (n=0 maps to None, n>0
 * maps to Some(n)). Prefer `tdx_config_set_decode_threads_explicit`
 * for new code.
 */
int32_t tdx_config_set_decode_threads(TdxConfig* config, size_t n);

/**
 * Legacy n-only setter for decode_queue_depth. Prefer
 * `tdx_config_set_decode_queue_depth_explicit` for new code.
 */
int32_t tdx_config_set_decode_queue_depth(TdxConfig* config, size_t n);

/* ── Client ── */

/** Connect to ThetaData servers. Returns NULL on connection/auth failure. */
TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Free a client handle. */
void tdx_client_free(TdxClient* client);

/* ── String free ── */

/** Free a string returned by any tdx_* function. */
void tdx_string_free(char* s);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  REST routing policy                                                   */
/*                                                                        */
/*  Routes the four historical-quote endpoints (option_history_quote,     */
/*  option_history_trade_quote, option_history_greeks_implied_volatility, */
/*  option_history_greeks_first_order) over the local Terminal's REST     */
/*  API when the caller wants a single transport for every quote-bearing  */
/*  call. Disabled by default; install via tdx_config_with_rest_fallback. */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Opaque fallback-policy handle. Construct via one of the factories,
 *  install on a config via tdx_config_with_rest_fallback, free with
 *  tdx_fallback_policy_free. */
typedef struct TdxFallbackPolicy TdxFallbackPolicy;

/** Disabled -- no REST routing. Identical to never calling
 *  tdx_config_with_rest_fallback. */
TdxFallbackPolicy* tdx_fallback_policy_disabled(void);

/** Always route the historical-quote endpoints over REST regardless of date. */
TdxFallbackPolicy* tdx_fallback_policy_rest_always(const char* base_url);

/** Free a fallback policy handle. */
void tdx_fallback_policy_free(TdxFallbackPolicy* policy);

/** Install the given fallback policy on a config. Borrows policy (clones
 *  the inner enum); the caller retains ownership and must still free policy
 *  via tdx_fallback_policy_free. Returns 0 on success, -1 on null-pointer
 *  error (check tdx_last_error()). */
int tdx_config_with_rest_fallback(TdxConfig* config, const TdxFallbackPolicy* policy);

/* ── Historical _with_fallback shims ── */

/** Fetch option NBBO history per the configured FallbackPolicy.
 *  symbol, expiration, start_date are required; end_date, strike, right,
 *  interval may be NULL to omit. Returns empty array on error; check
 *  tdx_last_error(). Caller must free via tdx_quote_tick_array_free. */
TdxQuoteTickArray tdx_option_history_quote_with_fallback(
    const TdxClient* client,
    const char* symbol, const char* expiration, const char* start_date,
    const char* end_date, const char* strike, const char* right, const char* interval);

/** Fetch combined trade+quote history per the configured FallbackPolicy.
 *  Same signature contract as tdx_option_history_quote_with_fallback
 *  (minus `interval`). Caller must free via tdx_trade_quote_tick_array_free. */
TdxTradeQuoteTickArray tdx_option_history_trade_quote_with_fallback(
    const TdxClient* client,
    const char* symbol, const char* expiration, const char* start_date,
    const char* end_date, const char* strike, const char* right);

/** Fetch implied-volatility history per the configured FallbackPolicy.
 *  Caller must free via tdx_iv_tick_array_free. */
TdxIvTickArray tdx_option_history_greeks_implied_volatility_with_fallback(
    const TdxClient* client,
    const char* symbol, const char* expiration, const char* start_date,
    const char* end_date, const char* strike, const char* right, const char* interval);

/** Fetch first-order Greeks history per the configured FallbackPolicy.
 *  Caller must free via tdx_greeks_first_order_tick_array_free. */
TdxGreeksFirstOrderTickArray tdx_option_history_greeks_first_order_with_fallback(
    const TdxClient* client,
    const char* symbol, const char* expiration, const char* start_date,
    const char* end_date, const char* strike, const char* right, const char* interval);

/* Generated option-aware endpoint declarations. */
#include "endpoint_with_options.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Greeks (standalone)                                                   */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Compute all 23 Greeks + IV. `right` accepts "C"/"P" or "call"/"put" (case-insensitive).
 *  Returns heap-allocated TdxGreeksResult (or NULL on error). Caller must free with tdx_greeks_result_free. */
TdxGreeksResult* tdx_all_greeks(double spot, double strike, double rate, double div_yield,
                                double tte, double option_price, const char* right);

/** Compute implied volatility. `right` accepts "C"/"P" or "call"/"put" (case-insensitive).
 *  Returns 0 on success, -1 on failure. */
int tdx_implied_volatility(double spot, double strike, double rate, double div_yield,
                           double tte, double option_price, const char* right,
                           double* out_iv, double* out_error);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Cross-language utility helpers — conditions / exchange / sequences   */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All `tdx_*_name` / `tdx_*_description` / `tdx_exchange_*` returns are
 * NUL-terminated UTF-8 C strings owned by the library. They are
 * `'static`-lifetime — DO NOT FREE. The pointer remains valid for the
 * lifetime of the process.  Unknown codes return either "UNKNOWN" (name
 * lookup) or "" (description lookup), never NULL. */

/** Trade condition name lookup. Returns "UNKNOWN" for unrecognised codes. */
const char* tdx_condition_name(int32_t code);

/** Trade condition description lookup. Returns "" for unrecognised codes. */
const char* tdx_condition_description(int32_t code);

/** True if the trade condition code represents a cancellation. */
bool tdx_condition_is_cancel(int32_t code);

/** True if the trade condition code updates the volume bar. */
bool tdx_condition_updates_volume(int32_t code);

/** Quote condition name lookup. Returns "UNKNOWN" for unrecognised codes. */
const char* tdx_quote_condition_name(int32_t code);

/** Quote condition description lookup. Returns "" for unrecognised codes. */
const char* tdx_quote_condition_description(int32_t code);

/** True if the quote condition is firm (binding). */
bool tdx_quote_condition_is_firm(int32_t code);

/** True if the quote condition indicates a trading halt. */
bool tdx_quote_condition_is_halted(int32_t code);

/** Exchange name lookup (e.g. 3 -> "NewYorkStockExchange"). */
const char* tdx_exchange_name(int32_t code);

/** Exchange MIC-like symbol lookup (e.g. 3 -> "NYSE"). */
const char* tdx_exchange_symbol(int32_t code);

/** Convert a signed wire-encoded trade-sequence value to its unsigned
 *  monotonic form. */
uint64_t tdx_sequence_signed_to_unsigned(int64_t signed_value);

/** Convert an unsigned monotonic trade-sequence value back to its signed
 *  wire encoding. */
int64_t tdx_sequence_unsigned_to_signed(uint64_t unsigned_value);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Streaming — #[repr(C)] event types                                    */
/* ═══════════════════════════════════════════════════════════════════════ */

/* Streaming event structs are schema-driven. The include below pulls in
 * the same typedefs the Go SDK uses, generated at build time — so the
 * C++ header can never drift from the Rust `#[repr(C)]` layout again.
 * See `thetadx.hpp` for `static_assert(offsetof)` guards that fail the
 * build at compile time if the schema and the C++ consumer ever
 * disagree.
 *
 * Each variant is a typed `#[repr(C)]` struct. Consumers dispatch via
 * `event->kind` and read the matching `event-><variant>` payload —
 * for example
 *
 *   if (event->kind == TDX_FPSS_LOGIN_SUCCESS)
 *       printf("perms=%s\n", event->login_success.permissions);
 *   if (event->kind == TDX_FPSS_DISCONNECTED)
 *       printf("reason=%d\n", event->disconnected.reason);
 *
 * Borrowed pointers (`Contract.symbol`, `LoginSuccess.permissions`,
 * `ServerError.message`, `Error.message`, `Ping.payload`,
 * `UnknownFrame.payload`) are valid only for the duration of the
 * user callback — copy out before returning. Do NOT free. */
#include "fpss_event_structs.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Real-time streaming client                                            */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Connect to the real-time streaming servers. Returns NULL on failure. */
TdxFpssHandle* tdx_fpss_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Polymorphic subscribe / unsubscribe — see TdxSubscriptionRequest below. */

/** Check if authenticated. Returns 1 if true, 0 if false. */
int tdx_fpss_is_authenticated(const TdxFpssHandle* h);

/** Get active subscriptions as typed array. Caller must free with tdx_subscription_array_free. */
TdxSubscriptionArray* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);

/** User callback signature for tdx_*_set_callback.
 *  `event` is valid only for the duration of the call -- copy any fields the
 *  caller wants to outlive the callback. `ctx` is the opaque pointer the
 *  caller registered alongside the callback; it is passed back unchanged. */
typedef void (*TdxFpssCallback)(const TdxFpssEvent* event, void* ctx);

/** Register a streaming callback and open the streaming connection.
 *
 *  Events flow `streaming reader -> bounded ring -> consumer thread ->
 *  catch_unwind(callback)`. The reader thread NEVER blocks on user code:
 *  on ring overflow events are dropped and counted (tdx_fpss_dropped_events).
 *
 *  ## ctx lifetime + thread affinity
 *
 *  `ctx` MUST remain valid until ONE of: (a) tdx_fpss_free() returns
 *  (which performs shutdown if needed and applies the drain barrier
 *  internally with a 5 s timeout), or (b) tdx_fpss_shutdown() /
 *  tdx_fpss_reconnect() returns AND tdx_fpss_await_drain() has
 *  returned 1. The consumer thread accesses ctx on every event and on
 *  every tdx_fpss_reconnect(), serially on a single thread. Freeing ctx
 *  without one of these barriers is undefined behavior.
 *
 *  The consumer thread invokes `callback(event, ctx)` serially on
 *  a single thread. The user does NOT need internal locks for callback-
 *  private state.
 *
 *  ## Lifecycle contract (one-shot rule)
 *
 *  Must be called exactly ONCE per handle. After tdx_fpss_shutdown() this
 *  handle is terminal: a second register, a register-after-shutdown, a
 *  reconnect-after-shutdown, or a double-shutdown all return -1 with a
 *  clear tdx_last_error() string ("streaming callback already installed -- ..."
 *  or "streaming handle has already been shut down -- this is terminal").
 *
 *  This is intentionally stricter than tdx_unified_set_callback(), where
 *  set-after-stop is supported as a normal user flow.
 *
 *  Returns 0 on success, -1 on error (check tdx_last_error()). */
int tdx_fpss_set_callback(const TdxFpssHandle* h, TdxFpssCallback callback, void* ctx);

/** Reconnect the streaming session using the previously-registered
 *  callback. Returns 0 or -1. Returns -1 with "streaming handle has
 *  already been shut down -- this is terminal" if the handle is past
 *  tdx_fpss_shutdown. */
int tdx_fpss_reconnect(const TdxFpssHandle* h);

/** Cumulative count of streaming events the TLS reader could not publish
 *  into the bounded ring because the consumer fell behind and the ring
 *  was full. Returns 0 if the handle is null or no callback has been
 *  installed yet. */
uint64_t tdx_fpss_dropped_events(const TdxFpssHandle* h);

/** Shut down the streaming client. Terminal: every subsequent
 *  set_callback / reconnect / shutdown call on this handle returns -1
 *  with a clear tdx_last_error() string. The handle remains valid for
 *  tdx_fpss_free() only. Returns asynchronously: the streaming reader
 *  and consumer continue draining in-flight events through the
 *  registered callback until they observe the shutdown signal and
 *  exit. Pair with tdx_fpss_await_drain() (or use tdx_fpss_free(), which
 *  applies the drain barrier internally) before freeing the callback
 *  ctx. */
void tdx_fpss_shutdown(const TdxFpssHandle* h);

/** Wait for the previously-superseded streaming session to quiesce.
 *
 *  Returns 1 once the previous tdx_fpss_reconnect / tdx_fpss_shutdown
 *  session's consumer has finished firing the registered callback.
 *  Returns 0 on timeout or when no session has been superseded on this
 *  handle.
 *
 *  Must be called from a thread other than the streaming consumer
 *  thread; calling it from inside the user callback would block the
 *  helper the consumer is waiting on and always time out. */
int tdx_fpss_await_drain(const TdxFpssHandle* h, uint64_t timeout_ms);

/** Free the streaming handle.
 *
 *  Accepts the handle in either lifecycle state: if shutdown has not
 *  yet been called, tdx_fpss_free performs the shutdown sequence
 *  itself. Returns only after the consumer thread has finished firing
 *  the registered callback (internal 5-second drain barrier). On
 *  drain-flag timeout, emits a tracing::error! and proceeds with
 *  destruction; in that diagnostic case the consumer may still be
 *  firing, so user code must keep ctx valid past return. Under normal
 *  operation drain completes in low single-digit milliseconds, so ctx
 *  is safe to free immediately on return. */
void tdx_fpss_free(TdxFpssHandle* h);

/* ======================================================================= */
/*  Unified client -- historical + streaming through one handle            */
/* ======================================================================= */

/** Connect to ThetaData (historical only -- real-time streaming is NOT started).
 *  Returns NULL on connection/auth failure (check tdx_last_error()). */
TdxUnified* tdx_unified_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Register a streaming callback and start streaming on the unified client.
 *
 *  Events flow `streaming reader -> bounded ring -> consumer thread ->
 *  catch_unwind(callback)`. Reader never blocks on user code; ring-overflow
 *  events are dropped (tdx_unified_dropped_events).
 *
 *  ## ctx lifetime + thread affinity
 *
 *  `ctx` MUST remain valid until ONE of: (a) tdx_unified_free()
 *  returns (which calls stop_streaming and applies the drain barrier
 *  internally with a 5 s timeout), (b) tdx_unified_stop_streaming() /
 *  tdx_unified_reconnect() returns AND tdx_unified_await_drain() has
 *  returned 1, or (c) a successful replacement tdx_unified_set_callback
 *  has returned AND tdx_unified_await_drain() has returned 1 for the
 *  prior session. The consumer thread accesses ctx on every event and
 *  reconnect, serially on a single thread. Freeing ctx without one of
 *  these barriers is undefined behavior.
 *
 *  ## Lifecycle contract (REPLACEMENT after stop)
 *
 *  Unlike tdx_fpss_set_callback (one-shot), the unified path supports
 *  stop+register as a normal user flow: after tdx_unified_stop_streaming
 *  another tdx_unified_set_callback REPLACES the saved (callback, ctx).
 *  tdx_unified_reconnect is built on top of this. Calling set_callback
 *  while streaming is already active returns -1 with "streaming already
 *  started".
 *
 *  Returns 0 on success, -1 on error. */
int tdx_unified_set_callback(const TdxUnified* handle, TdxFpssCallback callback, void* ctx);

/** Subscription request scope discriminator (TdxSubscriptionRequest.scope). */
#define TDX_SUB_SCOPE_CONTRACT 0
#define TDX_SUB_SCOPE_FULL     1

/** Subscription kind discriminator (TdxSubscriptionRequest.kind). */
#define TDX_SUB_KIND_QUOTE         0
#define TDX_SUB_KIND_TRADE         1
#define TDX_SUB_KIND_OPEN_INTEREST 2

/** Polymorphic subscribe / unsubscribe request payload.
 *
 * Mirrors the Rust `Subscription` enum across the C ABI.
 *
 * - Per-contract stock: scope=CONTRACT, symbol="AAPL", option fields NULL.
 * - Per-contract option: scope=CONTRACT, symbol="SPY", expiration / strike / right set.
 * - Full-stream: scope=FULL, sec_type="OPTION" (or "STOCK", "INDEX"), per-contract fields NULL.
 */
typedef struct {
    int32_t scope;            /* TDX_SUB_SCOPE_CONTRACT or TDX_SUB_SCOPE_FULL */
    int32_t kind;             /* TDX_SUB_KIND_QUOTE / _TRADE / _OPEN_INTEREST */
    const char* symbol;       /* per-contract only */
    const char* expiration;   /* per-contract option only */
    const char* strike;       /* per-contract option only */
    const char* right;        /* per-contract option only */
    const char* sec_type;     /* full-stream only */
} TdxSubscriptionRequest;

/** Polymorphic subscribe on the unified client. Returns 0 or -1. */
int tdx_unified_subscribe(const TdxUnified* handle, const TdxSubscriptionRequest* request);

/** Polymorphic unsubscribe on the unified client. Returns 0 or -1. */
int tdx_unified_unsubscribe(const TdxUnified* handle, const TdxSubscriptionRequest* request);

/** Polymorphic subscribe on the standalone streaming client. Returns 0 or -1. */
int tdx_fpss_subscribe(const TdxFpssHandle* h, const TdxSubscriptionRequest* request);

/** Polymorphic unsubscribe on the standalone streaming client. Returns 0 or -1. */
int tdx_fpss_unsubscribe(const TdxFpssHandle* h, const TdxSubscriptionRequest* request);

/** Reconnect unified streaming, re-subscribing all previous subscriptions. Returns 0 or -1. */
int tdx_unified_reconnect(const TdxUnified* handle);

/** Check if streaming is active. Returns 1 if streaming, 0 otherwise. */
int tdx_unified_is_streaming(const TdxUnified* handle);

/** Get active subscriptions as typed array. Caller must free with tdx_subscription_array_free. */
TdxSubscriptionArray* tdx_unified_active_subscriptions(const TdxUnified* handle);

/** Get active full-stream subscriptions as typed array. Each entry's
 *  `contract` field carries the security-type discriminant
 *  ("Stock" / "Option" / "Index") the full-stream subscription is bound
 *  to; the `kind` field is the subscription kind discriminant
 *  ("Trade" / "OpenInterest" / "Quote"). Returns null on error.
 *  Caller must free with tdx_subscription_array_free. */
TdxSubscriptionArray* tdx_unified_active_full_subscriptions(const TdxUnified* handle);

/** Borrow the historical client from a unified handle. Do NOT free the returned pointer. */
const TdxClient* tdx_unified_historical(const TdxUnified* handle);

/** Stop streaming on the unified client. Historical remains available.
 *  Returns asynchronously: the streaming reader and consumer continue
 *  draining in-flight events through the registered callback until they
 *  observe the shutdown signal. Pair with tdx_unified_await_drain()
 *  (or use tdx_unified_free(), which applies the drain barrier
 *  internally) before freeing the callback ctx. */
void tdx_unified_stop_streaming(const TdxUnified* handle);

/** Wait for the previously-superseded streaming session to quiesce.
 *
 *  Returns 1 once the previous consumer thread has finished firing the
 *  registered callback. Returns 0 on timeout or when no stream has ever
 *  been started or stopped on this handle.
 *
 *  Must be called from a thread other than the streaming consumer thread. */
int tdx_unified_await_drain(const TdxUnified* handle, uint64_t timeout_ms);

/** Cumulative count of streaming events the TLS reader could not publish
 *  into the bounded ring because the consumer fell behind and the ring
 *  was full. Returns 0 if the handle is null or no callback has been
 *  installed yet. */
uint64_t tdx_unified_dropped_events(const TdxUnified* handle);

/** Free a unified client handle.
 *
 *  Calls tdx_unified_stop_streaming internally, then waits up to 5
 *  seconds for the consumer thread to finish firing the registered
 *  callback before destroying the handle. On drain-flag timeout,
 *  emits a tracing::error! and proceeds with destruction; in that
 *  diagnostic case the consumer may still be firing, so user code
 *  must keep ctx valid past return. Under normal operation drain
 *  completes in low single-digit milliseconds, so ctx is safe to
 *  free immediately on return. */
void tdx_unified_free(TdxUnified* handle);

/* ── Pull-iter delivery ─────────────────────────────────────
 *
 * Sibling of the push-callback path. `tdx_unified_set_callback` sends
 * each event through a user `extern "C" fn` invoked on the streaming
 * consumer thread; the iterator instead drains a per-client bounded
 * queue from the caller's own thread, so the consumer thread is
 * decoupled from any per-event GIL / event-loop costs the binding
 * pays. Mutually exclusive with the callback path on the same
 * `TdxUnified*`; switch by stopping streaming and starting again.
 */

/** Opaque pull-iter handle returned by tdx_unified_start_streaming_iter. */
typedef struct TdxFpssEventIterator TdxFpssEventIterator;

/** Start real-time streaming on the unified client in pull-iter mode.
 *
 *  Returns a freshly allocated `TdxFpssEventIterator*` on success.
 *  Mutually exclusive with `tdx_unified_set_callback` — calling
 *  either while streaming is already running returns NULL with
 *  `tdx_last_error()` set to `"streaming already started"`. Free with
 *  `tdx_fpss_event_iter_free` when done iterating.
 *
 *  Returns NULL on connection / auth / state failure. */
TdxFpssEventIterator* tdx_unified_start_streaming_iter(const TdxUnified* handle);

/** Pop the next streaming event into `*out_event`. `timeout_ms = 0` is a
 *  non-blocking poll; positive `timeout_ms` blocks up to that
 *  deadline.
 *
 *  Return values:
 *  -  0 — event filled into `*out_event`.
 *  -  1 — timeout expired with no event; `*out_event` untouched.
 *  - -1 — terminal end-of-stream (queue drained on a stopped session)
 *         OR call-site error (check `tdx_last_error()`).
 *
 *  The borrowed pointer fields inside `*out_event` (`Contract.symbol`,
 *  `LoginSuccess.permissions`, payload byte slices, etc.) reference
 *  heap memory owned by the iterator handle's internal buffer. They
 *  are valid until the next `tdx_fpss_event_iter_next` call OR until
 *  `tdx_fpss_event_iter_free` is invoked, whichever happens first.
 *  Copy any fields the consumer wants to outlive the next call. */
int tdx_fpss_event_iter_next(TdxFpssEventIterator* it,
                             TdxFpssEvent* out_event,
                             int32_t timeout_ms);

/** Mark the iterator closed. Subsequent `_next` calls return -1
 *  (terminal) once the queue is drained, without shutting down the
 *  underlying streaming session. Idempotent. */
void tdx_fpss_event_iter_close(TdxFpssEventIterator* it);

/** Free a pull-iter handle. Does NOT stop the underlying streaming
 *  session — call `tdx_unified_stop_streaming` first if you need a
 *  full shutdown. */
void tdx_fpss_event_iter_free(TdxFpssEventIterator* it);

/* ── FLATFILES surface ────────────────────────────────────────────────
 *
 * Whole-universe daily snapshots over the legacy historical-channel
 * port. The schema is determined at runtime by (sec_type, req_type),
 * so the typed decoder returns an opaque row-list handle that you
 * serialise to Arrow IPC bytes when you want columnar output.
 */

/** Opaque handle wrapping a decoded `Vec<FlatFileRow>`. Created by
 *  tdx_flatfile_request_decoded; freed by tdx_flatfile_rowlist_free. */
typedef struct TdxFlatFileRowList TdxFlatFileRowList;

/** Heap-owned byte buffer (Arrow IPC stream) returned by
 *  tdx_flatfile_rows_to_arrow_ipc. Caller MUST free with
 *  tdx_flatfile_bytes_free. */
typedef struct TdxFlatFileBytes {
    const uint8_t* data;
    size_t len;
} TdxFlatFileBytes;

/** Pull a decoded flat-file blob for (sec_type, req_type, date) and
 *  return an opaque row-list handle.
 *
 *  sec_type  -- "OPTION" / "STOCK" / "INDEX"
 *  req_type  -- "EOD" / "QUOTE" / "OPEN_INTEREST" / "OHLC" / "TRADE" /
 *               "TRADE_QUOTE"
 *  date      -- "YYYYMMDD"
 *
 *  Returns NULL on error; check tdx_last_error(). The returned handle
 *  MUST be freed with tdx_flatfile_rowlist_free. */
TdxFlatFileRowList* tdx_flatfile_request_decoded(
    const TdxUnified* handle,
    const char* sec_type,
    const char* req_type,
    const char* date);

/** Number of rows in a row-list handle. Returns 0 if rowlist is NULL. */
size_t tdx_flatfile_rows_count(const TdxFlatFileRowList* rowlist);

/** Serialise the row list as Arrow IPC stream bytes. The schema is
 *  inferred from the first row by `flatfiles::arrow::rows_to_arrow`.
 *
 *  Returns (data=NULL, len=0) on error; check tdx_last_error().
 *  Caller MUST free the returned bytes with tdx_flatfile_bytes_free. */
TdxFlatFileBytes tdx_flatfile_rows_to_arrow_ipc(
    const TdxFlatFileRowList* rowlist);

/** Free a byte buffer returned by tdx_flatfile_rows_to_arrow_ipc. */
void tdx_flatfile_bytes_free(TdxFlatFileBytes bytes);

/** Free a row-list handle returned by tdx_flatfile_request_decoded. */
void tdx_flatfile_rowlist_free(TdxFlatFileRowList* rowlist);

/** Pull a flat-file blob and write the requested vendor format
 *  ("csv" / "jsonl") directly to `path`. Returns 0 on success, -1 on
 *  error; check tdx_last_error(). The format extension is appended to
 *  `path` automatically if missing. */
int tdx_flatfile_request_to_path(
    const TdxUnified* handle,
    const char* sec_type,
    const char* req_type,
    const char* date,
    const char* path,
    const char* format);

#ifdef __cplusplus
}
#endif

#endif /* THETADX_H */
