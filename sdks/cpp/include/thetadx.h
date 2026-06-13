/**
 * thetadatadx C FFI header.
 *
 * This header declares the C interface to the thetadatadx Rust SDK.
 * Used by both the C++ wrapper and any other C-compatible language.
 *
 * Memory model:
 * - Opaque handles (TdxCredentials*, TdxMddsClient*, TdxConfig*) are heap-allocated
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
typedef struct TdxMddsClient TdxMddsClient;
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

/* Calendar day-type codes carried by TdxCalendarDay.status — the
 * vendor's own vocabulary. Resolve the text form with
 * tdx_calendar_status_name(). */
#define TDX_CALENDAR_STATUS_OPEN 0
#define TDX_CALENDAR_STATUS_EARLY_CLOSE 1
#define TDX_CALENDAR_STATUS_FULL_CLOSE 2
#define TDX_CALENDAR_STATUS_WEEKEND 3

TDX_ALIGN64_BEGIN typedef struct {
    int32_t date;
    /* C99 bool (1 byte): whether the market trades at all on this date
     * (true for open and early-close days). 3 bytes padding follow. */
    bool is_open;
    int32_t open_time;
    int32_t close_time;
    /* One of the TDX_CALENDAR_STATUS_* codes; string form via
     * tdx_calendar_status_name(). */
    int32_t status;
    uint8_t _tail_padding[44];
} TdxCalendarDay TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    /* EOD report creation time (NOT a trade time), ms since midnight ET. */
    int32_t created_ms_of_day;
    /* Time of the day's last trade, ms since midnight ET. 0 when no
     * trades printed that day (open/high/low/close are 0.0 then too). */
    int32_t last_trade_ms_of_day;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[44];
} TdxOhlcTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t open_interest;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
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
    int32_t expiration;     /* YYYYMMDD */
    /* 4 bytes padding before f64 */
    double strike;          /* dollars */
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put. Cast to char for display. */
    uint32_t right;
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

/* ── Arrow IPC terminal for history tick rows ── */

/* Heap-owned byte buffer (Arrow IPC stream) returned by the per-tick
 * tdx_*_to_arrow_ipc terminals. Caller MUST free with tdx_arrow_bytes_free.
 * Layout-identical to TdxFlatFileBytes. */
typedef struct TdxArrowBytes {
    const uint8_t* data;
    size_t len;
} TdxArrowBytes;

/* Serialise a span of history tick rows as an Arrow IPC stream — the same
 * columnar exit Python exposes via <TickName>List.to_arrow(). `rows` may be
 * NULL only when `len` is 0 (a valid zero-row stream). Returns (data=NULL,
 * len=0) on error with tdx_last_error() set; on success the caller MUST free
 * the bytes with tdx_arrow_bytes_free. The element type is the layout-pinned
 * tick struct the matching history endpoint returns. */
TdxArrowBytes tdx_eod_ticks_to_arrow_ipc(const TdxEodTick* rows, size_t len);
TdxArrowBytes tdx_ohlc_ticks_to_arrow_ipc(const TdxOhlcTick* rows, size_t len);
TdxArrowBytes tdx_trade_ticks_to_arrow_ipc(const TdxTradeTick* rows, size_t len);
TdxArrowBytes tdx_quote_ticks_to_arrow_ipc(const TdxQuoteTick* rows, size_t len);
TdxArrowBytes tdx_greeks_all_ticks_to_arrow_ipc(const TdxGreeksAllTick* rows, size_t len);
TdxArrowBytes tdx_greeks_eod_ticks_to_arrow_ipc(const TdxGreeksEodTick* rows, size_t len);
TdxArrowBytes tdx_greeks_first_order_ticks_to_arrow_ipc(const TdxGreeksFirstOrderTick* rows, size_t len);
TdxArrowBytes tdx_greeks_second_order_ticks_to_arrow_ipc(const TdxGreeksSecondOrderTick* rows, size_t len);
TdxArrowBytes tdx_greeks_third_order_ticks_to_arrow_ipc(const TdxGreeksThirdOrderTick* rows, size_t len);
TdxArrowBytes tdx_trade_greeks_all_ticks_to_arrow_ipc(const TdxTradeGreeksAllTick* rows, size_t len);
TdxArrowBytes tdx_trade_greeks_first_order_ticks_to_arrow_ipc(const TdxTradeGreeksFirstOrderTick* rows, size_t len);
TdxArrowBytes tdx_trade_greeks_second_order_ticks_to_arrow_ipc(const TdxTradeGreeksSecondOrderTick* rows, size_t len);
TdxArrowBytes tdx_trade_greeks_third_order_ticks_to_arrow_ipc(const TdxTradeGreeksThirdOrderTick* rows, size_t len);
TdxArrowBytes tdx_trade_greeks_implied_volatility_ticks_to_arrow_ipc(const TdxTradeGreeksImpliedVolatilityTick* rows, size_t len);
TdxArrowBytes tdx_iv_ticks_to_arrow_ipc(const TdxIvTick* rows, size_t len);
TdxArrowBytes tdx_price_ticks_to_arrow_ipc(const TdxPriceTick* rows, size_t len);
TdxArrowBytes tdx_index_price_at_time_ticks_to_arrow_ipc(const TdxIndexPriceAtTimeTick* rows, size_t len);
TdxArrowBytes tdx_open_interest_ticks_to_arrow_ipc(const TdxOpenInterestTick* rows, size_t len);
TdxArrowBytes tdx_market_value_ticks_to_arrow_ipc(const TdxMarketValueTick* rows, size_t len);
TdxArrowBytes tdx_calendar_days_to_arrow_ipc(const TdxCalendarDay* rows, size_t len);
TdxArrowBytes tdx_interest_rate_ticks_to_arrow_ipc(const TdxInterestRateTick* rows, size_t len);
TdxArrowBytes tdx_trade_quote_ticks_to_arrow_ipc(const TdxTradeQuoteTick* rows, size_t len);

/** Free a byte buffer returned by any tdx_*_to_arrow_ipc terminal. */
void tdx_arrow_bytes_free(TdxArrowBytes bytes);

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

/** Server-supplied rate-limit back-off of the last FFI error on this
 *  thread, in milliseconds, or `-1` when the error carries no
 *  `RetryInfo` hint. Set only for a rate-limit error whose upstream
 *  status attached the hint; every other error reads `-1`. The C++
 *  `RateLimitError::retry_after()` surfaces this as a typed value. */
int64_t tdx_last_error_retry_after_ms(void);

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
#define TDX_ERR_INVALID_PARAMETER 13

/* ── Credentials ── */

/** Create credentials from email and password. Returns NULL on error. */
TdxCredentials* tdx_credentials_from_email(const char* email, const char* password);

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
 * Returns 0 on success, -1 on an invalid policy (outside {0, 1}) or null
 * config. A rejected policy sets tdx_last_error_code to
 * TDX_ERR_INVALID_PARAMETER so an unknown value is rejected with the
 * same typed class the Python / TypeScript bindings raise, never
 * silently coerced to Auto.
 */
int32_t tdx_config_set_reconnect_policy(TdxConfig* config, int policy);

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
 * Read the configured reconnect policy selector. Writes 0 (Auto),
 * 1 (Manual), or 2 (Custom) into *out_policy. Returns 0 on success,
 * -1 if either pointer is null.
 */
int32_t tdx_config_get_reconnect_policy(const TdxConfig* config, int32_t* out_policy);

/**
 * Read the generic-transient reconnect attempt budget (default 30).
 * When the policy is not Auto, writes the default-limits value.
 * Returns 0 on success, -1 if either pointer is null.
 */
int32_t tdx_config_get_reconnect_max_attempts(const TdxConfig* config, uint32_t* out);

/**
 * Read the rate-limited reconnect attempt budget (default 100). Same
 * shape as tdx_config_get_reconnect_max_attempts.
 */
int32_t tdx_config_get_reconnect_max_rate_limited_attempts(const TdxConfig* config,
                                                           uint32_t* out);

/**
 * Set the ServerRestarting reconnect attempt budget. Default 60. No
 * effect unless the reconnect policy is Auto.
 */
void tdx_config_set_reconnect_max_server_restart_attempts(TdxConfig* config, uint32_t n);

/**
 * Read the ServerRestarting reconnect attempt budget (default 60).
 * Same shape as tdx_config_get_reconnect_max_attempts.
 */
int32_t tdx_config_get_reconnect_max_server_restart_attempts(const TdxConfig* config,
                                                             uint32_t* out);

/**
 * Read the stable-window reset interval in seconds (default 60). Same
 * shape as tdx_config_get_reconnect_max_attempts.
 */
int32_t tdx_config_get_reconnect_stable_window_secs(const TdxConfig* config, uint64_t* out);

/**
 * Set the wall-clock reconnect envelope (seconds) for the
 * generic-transient and server-restart classes, measured from the
 * first attempt of a consecutive-reconnect sequence. 0 disables the
 * envelope (attempt budgets only). Default 300. No effect unless the
 * reconnect policy is Auto.
 */
void tdx_config_set_reconnect_max_elapsed_secs(TdxConfig* config, uint64_t secs);

/**
 * Read the wall-clock reconnect envelope in seconds (default 300;
 * 0 = disabled). Same shape as tdx_config_get_reconnect_max_attempts.
 */
int32_t tdx_config_get_reconnect_max_elapsed_secs(const TdxConfig* config, uint64_t* out);

/**
 * Set the cap (ms) on the exponential generic-transient reconnect
 * ladder. The ladder starts at reconnect_wait_ms and doubles per
 * consecutive attempt up to this value. Default 30_000.
 */
void tdx_config_set_reconnect_wait_max_ms(TdxConfig* config, uint64_t v);

/** Read the current reconnect wait_max_ms setting (default 30_000). */
int32_t tdx_config_get_reconnect_wait_max_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the flat reconnect cadence (ms) for ServerRestarting
 * disconnects. Default 5_000.
 */
void tdx_config_set_reconnect_wait_server_restart_ms(TdxConfig* config, uint64_t v);

/** Read the current reconnect wait_server_restart_ms setting (default 5_000). */
int32_t tdx_config_get_reconnect_wait_server_restart_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the jitter strategy applied to every reconnect delay.
 *   mode=0: Full (default) -- sample uniformly from [0, delay].
 *   mode=1: Equal -- delay/2 + uniform(0, delay/2).
 *   mode=2: Decorrelated -- walk relative to the previous delay.
 *   mode=3: None -- deterministic delays (tests only).
 * Returns 0 on success, -1 on an invalid mode or null config.
 */
int32_t tdx_config_set_reconnect_jitter(TdxConfig* config, int32_t mode);

/**
 * Read the configured reconnect jitter mode. Same encoding as
 * tdx_config_set_reconnect_jitter. Returns 0 on success, -1 if either
 * pointer is null.
 */
int32_t tdx_config_get_reconnect_jitter(const TdxConfig* config, int32_t* out_mode);

/**
 * Set the subscription-replay burst size used after an auto-reconnect:
 * frames are written in bursts of this many, each burst flushed and
 * followed by a jittered replay_pace_ms pause. Minimum 1 (validated at
 * connect). Default 50.
 */
void tdx_config_set_reconnect_replay_burst_size(TdxConfig* config, uint32_t n);

/** Read the current replay_burst_size setting (default 50). */
int32_t tdx_config_get_reconnect_replay_burst_size(const TdxConfig* config, uint32_t* out);

/**
 * Set the pause (ms) between subscription-replay bursts after an
 * auto-reconnect. 0 removes the pause. Default 5.
 */
void tdx_config_set_reconnect_replay_pace_ms(TdxConfig* config, uint64_t v);

/** Read the current replay_pace_ms setting (default 5). */
int32_t tdx_config_get_reconnect_replay_pace_ms(const TdxConfig* config, uint64_t* out);

/**
 * Reconnect-decision callback for tdx_config_set_reconnect_callback.
 * Invoked on the streaming I/O thread after each retriable involuntary
 * disconnect. reason is the RemoveReason discriminant; attempt is the
 * 1-based consecutive-reconnect counter. Return the reconnect delay in
 * milliseconds, or any negative value to stop reconnecting.
 */
typedef int64_t (*TdxReconnectCallback)(int32_t reason, uint32_t attempt, void* user_data);

/**
 * Install a custom reconnect policy driven by a C callback. Permanent
 * disconnect reasons never reach the callback. The callback runs on
 * the SDK's streaming I/O thread: cb and user_data must be safe to use
 * from another thread for as long as any client built from this config
 * is alive. Passing cb=NULL restores the default Auto policy. Returns
 * 0 on success, -1 if config is null.
 */
int32_t tdx_config_set_reconnect_callback(TdxConfig* config, TdxReconnectCallback cb,
                                          void* user_data);

/**
 * Set the FPSS read timeout (ms): the no-frames deadline after which
 * the streaming I/O loop declares the session dead and reconnects.
 * Default 3_000; validated to [100, 60_000] at connect.
 */
void tdx_config_set_fpss_timeout_ms(TdxConfig* config, uint64_t v);

/** Read the current fpss timeout_ms setting (default 3_000). */
int32_t tdx_config_get_fpss_timeout_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the per-server connect timeout (ms) for the streaming
 * connection. Default 2_000; validated to [1_000, 60_000] at connect.
 */
void tdx_config_set_fpss_connect_timeout_ms(TdxConfig* config, uint64_t v);

/** Read the current fpss connect_timeout_ms setting (default 2_000). */
int32_t tdx_config_get_fpss_connect_timeout_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the FPSS heartbeat ping interval (ms). Default 250; validated to
 * [100, 300_000] at connect.
 */
void tdx_config_set_fpss_ping_interval_ms(TdxConfig* config, uint64_t v);

/** Read the current fpss ping_interval_ms setting (default 250). */
int32_t tdx_config_get_fpss_ping_interval_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the per-iteration blocking-read slice (ms) for the streaming I/O
 * loop. Default 25; validated to [10, 500] at connect.
 */
void tdx_config_set_fpss_io_read_slice_ms(TdxConfig* config, uint64_t v);

/** Read the current fpss io_read_slice_ms setting (default 25). */
int32_t tdx_config_get_fpss_io_read_slice_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the last-frame watchdog (ms): when no frame of any kind has
 * arrived for this long the I/O loop force-reconnects. 0 disables.
 * Default 30_000.
 */
void tdx_config_set_fpss_data_watchdog_ms(TdxConfig* config, uint64_t v);

/** Read the current fpss data_watchdog_ms setting (default 30_000; 0 = disabled). */
int32_t tdx_config_get_fpss_data_watchdog_ms(const TdxConfig* config, uint64_t* out);

/**
 * Set the TCP keepalive idle time (seconds) before the first kernel
 * probe on a silent FPSS socket. Default 5; validated to [1, 7_200]
 * at connect.
 */
void tdx_config_set_fpss_keepalive_idle_secs(TdxConfig* config, uint64_t v);

/** Read the current fpss keepalive_idle_secs setting (default 5). */
int32_t tdx_config_get_fpss_keepalive_idle_secs(const TdxConfig* config, uint64_t* out);

/**
 * Set the interval (seconds) between TCP keepalive probes. Default 2;
 * validated to [1, 75] at connect.
 */
void tdx_config_set_fpss_keepalive_interval_secs(TdxConfig* config, uint64_t v);

/** Read the current fpss keepalive_interval_secs setting (default 2). */
int32_t tdx_config_get_fpss_keepalive_interval_secs(const TdxConfig* config, uint64_t* out);

/**
 * Set the number of unanswered TCP keepalive probes after which the
 * kernel declares the FPSS connection dead (where the platform exposes
 * the knob). Default 2; validated to [1, 10] at connect.
 */
void tdx_config_set_fpss_keepalive_retries(TdxConfig* config, uint32_t v);

/** Read the current fpss keepalive_retries setting (default 2). */
int32_t tdx_config_get_fpss_keepalive_retries(const TdxConfig* config, uint32_t* out);

/**
 * Set the FPSS event ring buffer size (slots). Must be a power of two
 * >= 64; invalid values are rejected at the setter (tdx_last_error).
 * Default 131_072.
 */
void tdx_config_set_fpss_ring_size(TdxConfig* config, size_t n);

/** Read the current fpss ring_size setting (default 131_072). */
int32_t tdx_config_get_fpss_ring_size(const TdxConfig* config, size_t* out);

/**
 * Set the FPSS host-selection policy.
 *   policy=0: Shuffled (default) -- fault-domain-aware per-client
 *             shuffle; a fleet spreads across hosts and consecutive
 *             failover attempts cross physical machines.
 *   policy=1: FixedOrder -- use the declared host order verbatim.
 * Returns 0 on success, -1 on an invalid policy or null config.
 */
int32_t tdx_config_set_fpss_host_selection(TdxConfig* config, int32_t policy);

/**
 * Read the configured FPSS host-selection policy. Same encoding as
 * tdx_config_set_fpss_host_selection. Returns 0 on success, -1 if
 * either pointer is null.
 */
int32_t tdx_config_get_fpss_host_selection(const TdxConfig* config, int32_t* out_policy);

/**
 * Set the FPSS host-shuffle seed using the (has_value, seed) widened
 * shape. has_value=false (default) derives a fresh per-client seed so
 * a fleet shuffles independently; has_value=true makes the shuffled
 * order deterministic -- useful for fleet sharding and tests. Ignored
 * under the FixedOrder policy. Returns 0 on success, -1 if config is
 * null.
 */
int32_t tdx_config_set_fpss_host_shuffle_seed_explicit(TdxConfig* config, bool has_value,
                                                       uint64_t seed);

/**
 * Read the current FPSS host-shuffle seed. *out_has_value=false
 * encodes the per-client-entropy sentinel. Returns 0 on success, -1
 * if any pointer is null.
 */
int32_t tdx_config_get_fpss_host_shuffle_seed(const TdxConfig* config, bool* out_has_value,
                                              uint64_t* out_seed);

/**
 * Set the wall-clock envelope (seconds) for one historical-channel
 * retry sequence, measured from the first attempt. 0 disables the
 * envelope (attempt budget only). Default 300.
 */
void tdx_config_set_retry_max_elapsed_secs(TdxConfig* config, uint64_t secs);

/** Read the current retry max_elapsed value in seconds (default 300; 0 = disabled). */
int32_t tdx_config_get_retry_max_elapsed_secs(const TdxConfig* config, uint64_t* out_secs);

/**
 * Toggle AWS-style full jitter on the flatfile retry ladder. Default
 * true; false gives the deterministic schedule, useful for tests.
 */
void tdx_config_set_flatfiles_jitter(TdxConfig* config, bool jitter);

/** Read the current flatfiles jitter setting (default true). */
int32_t tdx_config_get_flatfiles_jitter(const TdxConfig* config, bool* out_jitter);

/**
 * Set the async worker-thread count for embedded runtimes.
 *
 *   has_value=false: encodes the auto-size sentinel (`None`); `n`
 *     is ignored. Defers to the default sizing.
 *   has_value=true: encodes `Some(n)`. The builder clamps `n=0` to
 *     `1`, but the explicit `Some(0)` is preserved across the C
 *     boundary matching the widened Option shape so Python / TS / C++
 *     bindings agree.
 *
 * Returns 0 on success, -1 if `config` is NULL.
 */
int32_t tdx_config_set_worker_threads_explicit(TdxConfig* config, bool has_value, size_t n);

/**
 * Read the current async worker-thread count. Same widened
 * (has_value, n) shape:
 *
 *   *out_has_value=false: config holds `None` (auto-size). *out_n=0.
 *   *out_has_value=true:  config holds `Some(*out_n)`.
 *
 * Returns 0 on success, -1 if any pointer is null.
 */
int32_t tdx_config_get_worker_threads(const TdxConfig* config, bool* out_has_value, size_t* out_n);

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

/* ── AuthConfig field setters/getters ── */

/**
 * Set the Nexus auth URL on a config handle. `url` must be a non-null,
 * NUL-terminated, valid-UTF-8 C string. Returns 0 on success, -1 if
 * `config` is null or `url` is null / not valid UTF-8 (check
 * tdx_last_error()).
 */
int32_t tdx_config_set_nexus_url(TdxConfig* config, const char* url);

/**
 * Read auth.nexus_url. Returns a heap-owned NUL-terminated C string
 * the caller MUST release with tdx_string_free, or null on a null
 * handle / interior-NUL value (check tdx_last_error()).
 */
char* tdx_config_get_nexus_url(const TdxConfig* config);

/**
 * Set the QueryInfo.client_type identifier on a config handle.
 * `client_type` must be a non-null, NUL-terminated, valid-UTF-8 C
 * string. Returns 0 on success, -1 if `config` is null or
 * `client_type` is null / not valid UTF-8 (check tdx_last_error()).
 */
int32_t tdx_config_set_client_type(TdxConfig* config, const char* client_type);

/**
 * Read auth.client_type. Returns a heap-owned NUL-terminated C string
 * the caller MUST release with tdx_string_free, or null on a null
 * handle / interior-NUL value (check tdx_last_error()).
 */
char* tdx_config_get_client_type(const TdxConfig* config);

/* ── MetricsConfig field setter/getter ── */

/**
 * Set the Prometheus exporter port on a config handle. Uses the
 * widened (has_value, port) shape:
 *
 *   has_value=false: encodes `None` (exporter disabled). `port` ignored.
 *   has_value=true:  encodes `Some(port)`. The exporter binds an HTTP
 *     listener on 0.0.0.0:<port> when the metrics-prometheus feature
 *     is compiled in.
 *
 * Returns 0 on success, -1 if `config` is null.
 */
int32_t tdx_config_set_metrics_port(TdxConfig* config, bool has_value, uint16_t port);

/**
 * Read metrics.port. Same (has_value, port) shape:
 *
 *   *out_has_value=false: config holds `None` (disabled). *out_port=0.
 *   *out_has_value=true:  config holds `Some(*out_port)`.
 *
 * Returns 0 on success, -1 if any pointer is null.
 */
int32_t tdx_config_get_metrics_port(const TdxConfig* config, bool* out_has_value, uint16_t* out_port);

/**
 * Set streaming flush mode on a config handle.
 *   mode=0: Batched (default) -- flush only on PING every 100ms.
 *   mode=1: Immediate -- flush after every frame write.
 *
 * Returns 0 on success. Returns -1 and sets `tdx_last_error` /
 * `tdx_last_error_code = TDX_ERR_CONFIG` when `mode` is outside the
 * documented `{0, 1}` set or when `config` is null.
 */
int tdx_config_set_flush_mode(TdxConfig* config, int mode);

/**
 * Read the current streaming flush mode. Same encoding as
 * tdx_config_set_flush_mode: writes 0 (Batched) or 1 (Immediate) into
 * *out_mode. Returns 0 on success, -1 if either pointer is null.
 */
int32_t tdx_config_get_flush_mode(const TdxConfig* config, int32_t* out_mode);

/**
 * Set streaming OHLCVC derivation on a config handle.
 *   enabled=1 (default): derive OHLCVC bars locally from trade events.
 *   enabled=0: only emit server-sent OHLCVC frames (lower overhead).
 */
void tdx_config_set_derive_ohlcvc(TdxConfig* config, int enabled);

/**
 * Read the current OHLCVC-derivation flag. Writes true/false into
 * *out_enabled. Returns 0 on success, -1 if either pointer is null.
 */
int32_t tdx_config_get_derive_ohlcvc(const TdxConfig* config, bool* out_enabled);

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
 * Read the current concurrent in-flight gRPC request count. Writes the
 * value into *out_n (0 = auto-detect from the tier). Returns 0 on
 * success, -1 if either pointer is null.
 */
int32_t tdx_config_get_concurrent_requests(const TdxConfig* config, uint32_t* out_n);

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

/* ── MddsClient ── */

/** Connect a historical (MDDS) client to ThetaData servers. Returns NULL on
 *  connection/auth failure. */
TdxMddsClient* tdx_mdds_client_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Free a historical (MDDS) client handle. */
void tdx_mdds_client_free(TdxMddsClient* client);

/* ── String free ── */

/** Free a string returned by any tdx_* function. */
void tdx_string_free(char* s);

/* Generated option-aware endpoint declarations. */
#include "endpoint_with_options.h.inc"

/** User callback signature for the tdx_<endpoint>_stream server-stream entry
 *  points. Invoked once per decoded chunk drained from a historical result.
 *
 *  `rows` points at the first element of a contiguous run of `len` tick
 *  structs -- the SAME layout the matching tdx_<endpoint>_with_options array
 *  returns (e.g. a tdx_option_history_trade_stream chunk is `len` x
 *  TdxTradeTick). Cast `rows` to that tick pointer type before indexing. The
 *  pointer is valid only for the duration of the call -- copy any rows the
 *  caller wants to outlive the callback. An empty result drains as zero
 *  invocations (a null `rows` with `len == 0` is never delivered).
 *
 *  `ctx` is the opaque pointer registered alongside the callback; it is passed
 *  back unchanged on every invocation. */
typedef void (*TdxTickChunkCallback)(const void* rows, size_t len, void* ctx);

/* Generated server-stream endpoint declarations. */
#include "historical_stream.h.inc"

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

/** Calendar day-type vocabulary lookup (0 -> "open", 1 -> "early_close",
 *  2 -> "full_close", 3 -> "weekend"; "UNKNOWN" otherwise). 'static
 *  lifetime — DO NOT FREE. */
const char* tdx_calendar_status_name(int32_t code);

/** Combine an Eastern-Time YYYYMMDD date and milliseconds-of-day into
 *  Unix epoch milliseconds (UTC, DST-aware). Usable with any
 *  (date, *_ms_of_day) pair on the tick structs. Returns -1 when date
 *  is not a valid YYYYMMDD (including the 0 absent fill) or ms_of_day
 *  is outside 0..86,400,000. */
int64_t tdx_timestamp_ms(int32_t date, int32_t ms_of_day);

/** Convert a signed wire-encoded trade-sequence value to its unsigned
 *  monotonic form. `signed_value` must lie in the i32 wire range
 *  (-2,147,483,648 ..= 2,147,483,647). Writes the result to `out` and
 *  returns 0 on success; returns -1 and sets tdx_last_error_code to
 *  TDX_ERR_INVALID_PARAMETER when `signed_value` is outside the wire
 *  range or `out` is null, so an out-of-range value is rejected rather
 *  than silently reinterpreted. */
int32_t tdx_sequence_signed_to_unsigned(int64_t signed_value, uint64_t* out);

/** Convert an unsigned monotonic trade-sequence value back to its signed
 *  wire encoding. `unsigned_value` must lie in the unsigned wire range
 *  (0 ..= 2^32 - 1). Writes the result to `out` and returns 0 on
 *  success; returns -1 and sets tdx_last_error_code to
 *  TDX_ERR_INVALID_PARAMETER when `unsigned_value` is above the wire
 *  range or `out` is null. */
int32_t tdx_sequence_unsigned_to_signed(uint64_t unsigned_value, int64_t* out);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Streaming — #[repr(C)] event types                                    */
/* ═══════════════════════════════════════════════════════════════════════ */

/* Streaming event structs are schema-driven. The include below pulls in
 * the typedefs generated at build time from the same schema as the Rust
 * `#[repr(C)]` layout — so the C++ header can never drift from it again.
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

/** Read the option strike of a streaming TdxContract in dollars, folding
 *  the has_strike presence flag into the return value. TdxContract.strike
 *  already carries dollars; this surfaces the presence flag a plain field
 *  read would drop. Writes the dollar value to *out_dollars and returns
 *  true when the contract is an option; returns false (leaving *out_dollars
 *  untouched) for a non-option, null contract, or null output pointer.
 *  Mirrors the C++ tdx::strike(const TdxContract&) accessor. */
bool tdx_contract_strike_dollars(const TdxContract* contract, double* out_dollars);

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

/** Point-in-time count of streaming events published into the event ring
 *  but not yet drained into the registered callback — the in-flight depth
 *  between the I/O thread and the dispatcher. Rising occupancy that
 *  approaches tdx_fpss_ring_capacity predicts drops before
 *  tdx_fpss_dropped_events moves; sampling never blocks the feed and is
 *  safe from any thread. Returns 0 if the handle is null or has been shut
 *  down. */
uint64_t tdx_fpss_ring_occupancy(const TdxFpssHandle* h);

/** Configured capacity of the streaming event ring in slots (the
 *  fpss_ring_size setting, a power of two) — the fixed denominator for
 *  tdx_fpss_ring_occupancy. Returns 0 if the handle is null or has been
 *  shut down. */
uint64_t tdx_fpss_ring_capacity(const TdxFpssHandle* h);

/**
 * Milliseconds since the most recent inbound streaming frame of any
 * kind on this FPSS handle. Returns 0 on success with the value in
 * *out_ms, 1 when no session is live or no frame has been received
 * yet, -1 on a null pointer.
 */
int32_t tdx_fpss_millis_since_last_event(const TdxFpssHandle* h, uint64_t* out_ms);

/**
 * UNIX-nanosecond receive timestamp of the most recent inbound
 * streaming frame of any kind on this FPSS handle. Returns 0 when the
 * handle is null, no session is live, or no frame has been received
 * yet.
 */
int64_t tdx_fpss_last_event_received_at_unix_nanos(const TdxFpssHandle* h);

/**
 * Address (host:port) of the streaming server the current FPSS session
 * is connected to, following the session across auto-reconnects.
 * Returns a heap-owned C string the caller must release with
 * tdx_string_free, or NULL when no session is live.
 */
char* tdx_fpss_last_connected_addr(const TdxFpssHandle* h);



/** Cumulative count of user-callback panics caught by the per-invocation
 *  catch_unwind boundary since the current stream started. A panic in the
 *  callback is caught, recorded here, and does not stop event delivery —
 *  the next event continues normally. Returns 0 if the handle is null or
 *  no callback has been installed yet. Safe to call from any thread. */
uint64_t tdx_fpss_panic_count(const TdxFpssHandle* h);

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
#define TDX_SUB_KIND_MARKET_VALUE  3

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
    int32_t kind;             /* TDX_SUB_KIND_QUOTE / _TRADE / _OPEN_INTEREST / _MARKET_VALUE */
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
const TdxMddsClient* tdx_unified_historical(const TdxUnified* handle);

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

/** Point-in-time count of streaming events published into the event ring
 *  but not yet drained into the registered callback — the in-flight depth
 *  between the I/O thread and the dispatcher. Rising occupancy that
 *  approaches tdx_unified_ring_capacity predicts drops before
 *  tdx_unified_dropped_events moves; sampling never blocks the feed and is
 *  safe from any thread. Returns 0 if the handle is null or no callback
 *  has been installed yet. */
uint64_t tdx_unified_ring_occupancy(const TdxUnified* handle);

/** Configured capacity of the streaming event ring in slots (the
 *  fpss_ring_size setting, a power of two) — the fixed denominator for
 *  tdx_unified_ring_occupancy. Returns 0 if the handle is null or no
 *  callback has been installed yet. */
uint64_t tdx_unified_ring_capacity(const TdxUnified* handle);

/**
 * Milliseconds since the most recent inbound streaming frame of any
 * kind on this unified handle. Returns 0 on success with the value in
 * *out_ms, 1 when streaming has not started or no frame has been
 * received yet, -1 on a null pointer.
 */
int32_t tdx_unified_millis_since_last_event(const TdxUnified* handle, uint64_t* out_ms);

/**
 * UNIX-nanosecond receive timestamp of the most recent inbound
 * streaming frame of any kind on this unified handle. Returns 0 when
 * the handle is null, streaming has not started, or no frame has been
 * received yet.
 */
int64_t tdx_unified_last_event_received_at_unix_nanos(const TdxUnified* handle);

/**
 * Address (host:port) of the streaming server the current session is
 * connected to, following the session across auto-reconnects. Returns
 * a heap-owned C string the caller must release with tdx_string_free,
 * or NULL when streaming has not started.
 */
char* tdx_unified_last_connected_addr(const TdxUnified* handle);



/** Cumulative count of user-callback panics caught by the per-invocation
 *  catch_unwind boundary since the current stream started. A panic in the
 *  callback is caught, recorded here, and does not stop event delivery —
 *  the next event continues normally. Returns 0 if the handle is null or
 *  no callback has been installed yet. Safe to call from any thread. */
uint64_t tdx_unified_panic_count(const TdxUnified* handle);

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
