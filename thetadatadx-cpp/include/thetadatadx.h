/**
 * thetadatadx C FFI header.
 *
 * This header declares the C interface to the thetadatadx SDK.
 * Used by both the C++ wrapper and any other C-compatible language.
 *
 * Memory model:
 * - Opaque handles (ThetaDataDxCredentials*, ThetaDataDxMarketDataClient*, ThetaDataDxConfig*) are heap-allocated
 *   by the library and MUST be freed with the corresponding thetadatadx_*_free function.
 * - Tick data is returned as fixed-layout struct arrays. Each array type has a
 *   corresponding thetadatadx_*_array_free function that MUST be called.
 * - String arrays (ThetaDataDxStringArray) must be freed with thetadatadx_string_array_free.
 * - Functions that can fail return empty arrays (data=NULL, len=0) and set a
 *   thread-local error string retrievable via thetadatadx_last_error().
 */

#ifndef THETADATADX_H
#define THETADATADX_H

#include <stdint.h>
#include <stddef.h>
#ifndef __cplusplus
#include <stdbool.h>
#endif

#if defined(_MSC_VER)
#define THETADATADX_ALIGN64_BEGIN __declspec(align(64))
#define THETADATADX_ALIGN64_END
#else
#define THETADATADX_ALIGN64_BEGIN
#define THETADATADX_ALIGN64_END __attribute__((aligned(64)))
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ── Opaque handle types ── */
typedef struct ThetaDataDxCredentials ThetaDataDxCredentials;
typedef struct ThetaDataDxMarketDataClient ThetaDataDxMarketDataClient;
typedef struct ThetaDataDxConfig ThetaDataDxConfig;
typedef struct ThetaDataDxStreamHandle ThetaDataDxStreamHandle;
typedef struct ThetaDataDxClient ThetaDataDxClient;

/* Generated request-options bridge shared with the FFI surface. */
#include "endpoint_request_options.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Fixed-layout tick types                                               */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All tick structs are 64-byte aligned and carry explicit tail padding as
 * part of the ABI contract, so C/C++ array stepping stays byte-for-byte
 * compatible with the wire layout. Price fields are 64-bit doubles. */

/* Calendar day-type codes carried by ThetaDataDxCalendarDay.status — the
 * vendor's own vocabulary. Resolve the text form with
 * thetadatadx_calendar_status_name(). */
#define THETADATADX_CALENDAR_STATUS_OPEN 0
#define THETADATADX_CALENDAR_STATUS_EARLY_CLOSE 1
#define THETADATADX_CALENDAR_STATUS_FULL_CLOSE 2
#define THETADATADX_CALENDAR_STATUS_WEEKEND 3

/* Per-date trading-calendar entry (list_dates / calendar endpoints):
 * session open/close times and a THETADATADX_CALENDAR_STATUS_* day-type code. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t date;
    /* C99 bool (1 byte): whether the market trades at all on this date
     * (true for open and early-close days). 3 bytes padding follow. */
    bool is_open;
    int32_t open_time;
    int32_t close_time;
    /* One of the THETADATADX_CALENDAR_STATUS_* codes; string form via
     * thetadatadx_calendar_status_name(). */
    int32_t status;
    uint8_t _tail_padding[44];
} ThetaDataDxCalendarDay THETADATADX_ALIGN64_END;

/* End-of-day OHLC + closing-quote tick (*_history_eod) -- one row per
 * trading day fusing the day's open/high/low/close, volume/count, and
 * the closing bid/ask quote. */
THETADATADX_ALIGN64_BEGIN typedef struct {
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
    /* 4 bytes padding before the double field */
    double ask;
    int32_t ask_condition;
    int32_t date;
    int32_t expiration;
    /* 4 bytes padding before the double field */
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[4];
} ThetaDataDxEodTick THETADATADX_ALIGN64_END;

/* Full-union Greeks tick (option_*_greeks_all, interval-sampled). */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[20];
} ThetaDataDxGreeksAllTick THETADATADX_ALIGN64_END;

/* End-of-day Greeks tick (option_history_greeks_eod) -- fuses every
 * Greek with the twelve EOD trade/quote columns (open/high/low/close,
 * volume, count, bid_size, bid_exchange, bid_condition, ask_size,
 * ask_exchange, ask_condition) absent from the interval-sampled
 * GreeksAllTick. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[4];
} ThetaDataDxGreeksEodTick THETADATADX_ALIGN64_END;

/* First-order Greeks subset tick (option_*_greeks_first_order). */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[4];
} ThetaDataDxGreeksFirstOrderTick THETADATADX_ALIGN64_END;

/* Second-order Greeks subset tick (option_*_greeks_second_order). */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[12];
} ThetaDataDxGreeksSecondOrderTick THETADATADX_ALIGN64_END;

/* Third-order Greeks subset tick (option_*_greeks_third_order). The
 * vendor's third-order schema does not publish `vera`. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
    double bid;
    double ask;
    double speed;
    double zomma;
    double color;
    double ultima;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[20];
} ThetaDataDxGreeksThirdOrderTick THETADATADX_ALIGN64_END;

/* Per-OPRA-trade union Greeks tick (option_history_trade_greeks_all).
 * Carries the nine trade-side execution columns alongside every Greek
 * the server publishes -- distinct from the interval-sampled
 * ThetaDataDxGreeksAllTick whose rows carry the bid/ask quote pair instead. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[60];
} ThetaDataDxTradeGreeksAllTick THETADATADX_ALIGN64_END;

/* Per-OPRA-trade first-order Greeks tick
 * (option_history_trade_greeks_first_order). */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
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
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[44];
} ThetaDataDxTradeGreeksFirstOrderTick THETADATADX_ALIGN64_END;

/* Per-OPRA-trade second-order Greeks tick
 * (option_history_trade_greeks_second_order). */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
    double price;
    double gamma;
    double vanna;
    double charm;
    double vomma;
    double veta;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[52];
} ThetaDataDxTradeGreeksSecondOrderTick THETADATADX_ALIGN64_END;

/* Per-OPRA-trade third-order Greeks tick
 * (option_history_trade_greeks_third_order). The vendor's third-order
 * schema does not publish `vera`. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
    double price;
    double speed;
    double zomma;
    double color;
    double ultima;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[60];
} ThetaDataDxTradeGreeksThirdOrderTick THETADATADX_ALIGN64_END;

/* Per-OPRA-trade implied-volatility tick
 * (option_history_trade_greeks_implied_volatility). Carries only the
 * single `implied_volatility` + `iv_error` pair (NOT the bid/mid/ask IV
 * triple of the interval-sampled ThetaDataDxIvTick). */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
    double price;
    double implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[28];
} ThetaDataDxTradeGreeksImpliedVolatilityTick THETADATADX_ALIGN64_END;

/* InterestRateTick (2 fields). End-of-day interest rate (percent).
 * Wire shape per docs.thetadata.us/operations/interest_rate_history_eod.html:
 *   date  <- Text "YYYY-MM-DD" header `created`, parsed to a YYYYMMDD int32
 *   rate  <- Number percent (e.g. 4.36 for SOFR 2025-04-28)
 */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t date;
    /* 4 bytes padding before the double field */
    double rate;
    uint8_t _tail_padding[48];
} ThetaDataDxInterestRateTick THETADATADX_ALIGN64_END;

/* Interval-sampled implied-volatility tick (option_*_implied_volatility):
 * the bid/mid/ask quote with its bid/mid/ask IV triple. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
    double bid;
    double bid_implied_volatility;
    double midpoint;
    double implied_volatility;
    double ask;
    double ask_implied_volatility;
    double iv_error;
    int32_t underlying_ms_of_day;
    /* 4 bytes padding before the double field */
    double underlying_price;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[28];
} ThetaDataDxIvTick THETADATADX_ALIGN64_END;

/* Settlement market-value tick (option_*_market_value): the contract's
 * bid/ask and reference price used for daily mark-to-market. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
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
    uint8_t _tail_padding[12];
} ThetaDataDxMarketValueTick THETADATADX_ALIGN64_END;

/* OHLCVC bar tick (*_history_ohlc): one aggregated bar with
 * open/high/low/close, volume/count, and a SIP-rule VWAP. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
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
} ThetaDataDxOhlcTick THETADATADX_ALIGN64_END;

/* Open-interest tick (option_*_open_interest): the outstanding contract
 * count reported for the contract. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t open_interest;
    int32_t date;
    int32_t expiration;
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[36];
} ThetaDataDxOpenInterestTick THETADATADX_ALIGN64_END;

/* Bare index price tick (index_*_price): a single price stamped with
 * time and date, carrying no trade-side execution columns. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before the double field */
    double price;
    int32_t date;
    uint8_t _tail_padding[44];
} ThetaDataDxPriceTick THETADATADX_ALIGN64_END;

/* Trade-shaped index price tick (index_at_time_price) -- carries the
 * seven trade-side execution columns (sequence, ext_condition1..4,
 * condition, size, exchange) the bare ThetaDataDxPriceTick silently dropped,
 * including the SIP-exchange attribution field. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
    double price;
    int32_t date;
    uint8_t _tail_padding[12];
} ThetaDataDxIndexPriceAtTimeTick THETADATADX_ALIGN64_END;

/* NBBO quote tick (*_history_quote): the bid/ask quote with sizes,
 * exchanges, conditions, and a derived midpoint. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    /* 4 bytes padding before the double field */
    double bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    /* 4 bytes padding before the double field */
    double ask;
    int32_t ask_condition;
    int32_t date;
    int32_t expiration;
    /* 4 bytes padding before the double field */
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    /* 4 bytes padding before the double field */
    double midpoint;
    uint8_t _tail_padding[40];
} ThetaDataDxQuoteTick THETADATADX_ALIGN64_END;

/* Trade-with-quote tick (*_history_trade_quote): each trade print fused
 * with the bid/ask quote prevailing at execution time. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
    double price;
    int32_t condition_flags;
    int32_t price_flags;
    int32_t volume_type;
    int32_t records_back;
    int32_t quote_ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    /* 4 bytes padding before the double field */
    double bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    /* 4 bytes padding before the double field */
    double ask;
    int32_t ask_condition;
    int32_t date;
    int32_t expiration;
    /* 4 bytes padding before the double field */
    double strike;
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put, 0 when contract identity is absent
     * (single-contract queries). Cast to char for display. */
    uint32_t right;
    uint8_t _tail_padding[52];
} ThetaDataDxTradeQuoteTick THETADATADX_ALIGN64_END;

/* Single trade-print tick (*_history_trade): one OPRA/SIP execution with
 * price, size, exchange, sequence, and condition codes. */
THETADATADX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    /* 4 bytes padding before the double field */
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
    uint8_t _tail_padding[44];
} ThetaDataDxTradeTick THETADATADX_ALIGN64_END;

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Typed array return types                                              */
/* ═══════════════════════════════════════════════════════════════════════ */

/* Owned tick-array views: { const T* data; size_t len; }. Returned by the
 * matching thetadatadx_* data call; each MUST be freed with its thetadatadx_*_array_free
 * (below). An empty result is data=NULL, len=0. */
typedef struct { const ThetaDataDxEodTick* data; size_t len; } ThetaDataDxEodTickArray;
typedef struct { const ThetaDataDxOhlcTick* data; size_t len; } ThetaDataDxOhlcTickArray;
typedef struct { const ThetaDataDxTradeTick* data; size_t len; } ThetaDataDxTradeTickArray;
typedef struct { const ThetaDataDxQuoteTick* data; size_t len; } ThetaDataDxQuoteTickArray;
typedef struct { const ThetaDataDxGreeksAllTick* data; size_t len; } ThetaDataDxGreeksAllTickArray;
typedef struct { const ThetaDataDxGreeksEodTick* data; size_t len; } ThetaDataDxGreeksEodTickArray;
typedef struct { const ThetaDataDxGreeksFirstOrderTick* data; size_t len; } ThetaDataDxGreeksFirstOrderTickArray;
typedef struct { const ThetaDataDxGreeksSecondOrderTick* data; size_t len; } ThetaDataDxGreeksSecondOrderTickArray;
typedef struct { const ThetaDataDxGreeksThirdOrderTick* data; size_t len; } ThetaDataDxGreeksThirdOrderTickArray;
typedef struct { const ThetaDataDxTradeGreeksAllTick* data; size_t len; } ThetaDataDxTradeGreeksAllTickArray;
typedef struct { const ThetaDataDxTradeGreeksFirstOrderTick* data; size_t len; } ThetaDataDxTradeGreeksFirstOrderTickArray;
typedef struct { const ThetaDataDxTradeGreeksSecondOrderTick* data; size_t len; } ThetaDataDxTradeGreeksSecondOrderTickArray;
typedef struct { const ThetaDataDxTradeGreeksThirdOrderTick* data; size_t len; } ThetaDataDxTradeGreeksThirdOrderTickArray;
typedef struct { const ThetaDataDxTradeGreeksImpliedVolatilityTick* data; size_t len; } ThetaDataDxTradeGreeksImpliedVolatilityTickArray;
typedef struct { const ThetaDataDxIvTick* data; size_t len; } ThetaDataDxIvTickArray;
typedef struct { const ThetaDataDxPriceTick* data; size_t len; } ThetaDataDxPriceTickArray;
typedef struct { const ThetaDataDxIndexPriceAtTimeTick* data; size_t len; } ThetaDataDxIndexPriceAtTimeTickArray;
typedef struct { const ThetaDataDxOpenInterestTick* data; size_t len; } ThetaDataDxOpenInterestTickArray;
typedef struct { const ThetaDataDxMarketValueTick* data; size_t len; } ThetaDataDxMarketValueTickArray;
typedef struct { const ThetaDataDxCalendarDay* data; size_t len; } ThetaDataDxCalendarDayArray;
typedef struct { const ThetaDataDxInterestRateTick* data; size_t len; } ThetaDataDxInterestRateTickArray;
typedef struct { const ThetaDataDxTradeQuoteTick* data; size_t len; } ThetaDataDxTradeQuoteTickArray;

/* ── OptionContract (has heap-allocated symbol string) ── */

typedef struct {
    const char* symbol;     /* heap-allocated, freed with thetadatadx_option_contract_array_free */
    int32_t expiration;     /* YYYYMMDD */
    /* 4 bytes padding before the double field */
    double strike;          /* dollars */
    /* Unicode scalar value of the right character: 'C' (67) for a call,
     * 'P' (80) for a put. Cast to char for display. */
    uint32_t right;
} ThetaDataDxOptionContract;

typedef struct { const ThetaDataDxOptionContract* data; size_t len; } ThetaDataDxOptionContractArray;

/* ── String array (for list endpoints) ── */

typedef struct {
    const char* const* data;  /* array of NUL-terminated C strings */
    size_t len;
} ThetaDataDxStringArray;

/* ── Subscription types (active_subscriptions) ── */

typedef struct {
    const char* kind;      /* snake_case: per-contract "quote"/"trade"/"open_interest"/"market_value", full-stream "full_trades"/"full_open_interest" */
    const char* contract;  /* "SPY" or "SPY 20260417 550 C" */
} ThetaDataDxSubscription;

typedef struct {
    const ThetaDataDxSubscription* data;
    size_t len;
} ThetaDataDxSubscriptionArray;

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Free functions for typed arrays                                       */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Each frees the array returned by its matching thetadatadx_* data call and
 *  releases the backing allocation.
 *  @param arr The array returned by the matching data call; a NULL/empty
 *             (data=NULL, len=0) array is a no-op.
 *  @note Call exactly once per returned array. */
void thetadatadx_eod_tick_array_free(ThetaDataDxEodTickArray arr);
void thetadatadx_ohlc_tick_array_free(ThetaDataDxOhlcTickArray arr);
void thetadatadx_trade_tick_array_free(ThetaDataDxTradeTickArray arr);
void thetadatadx_quote_tick_array_free(ThetaDataDxQuoteTickArray arr);
void thetadatadx_greeks_all_tick_array_free(ThetaDataDxGreeksAllTickArray arr);
void thetadatadx_greeks_eod_tick_array_free(ThetaDataDxGreeksEodTickArray arr);
void thetadatadx_greeks_first_order_tick_array_free(ThetaDataDxGreeksFirstOrderTickArray arr);
void thetadatadx_greeks_second_order_tick_array_free(ThetaDataDxGreeksSecondOrderTickArray arr);
void thetadatadx_greeks_third_order_tick_array_free(ThetaDataDxGreeksThirdOrderTickArray arr);
void thetadatadx_trade_greeks_all_tick_array_free(ThetaDataDxTradeGreeksAllTickArray arr);
void thetadatadx_trade_greeks_first_order_tick_array_free(ThetaDataDxTradeGreeksFirstOrderTickArray arr);
void thetadatadx_trade_greeks_second_order_tick_array_free(ThetaDataDxTradeGreeksSecondOrderTickArray arr);
void thetadatadx_trade_greeks_third_order_tick_array_free(ThetaDataDxTradeGreeksThirdOrderTickArray arr);
void thetadatadx_trade_greeks_implied_volatility_tick_array_free(ThetaDataDxTradeGreeksImpliedVolatilityTickArray arr);
void thetadatadx_iv_tick_array_free(ThetaDataDxIvTickArray arr);
void thetadatadx_price_tick_array_free(ThetaDataDxPriceTickArray arr);
void thetadatadx_index_price_at_time_tick_array_free(ThetaDataDxIndexPriceAtTimeTickArray arr);
void thetadatadx_open_interest_tick_array_free(ThetaDataDxOpenInterestTickArray arr);
void thetadatadx_market_value_tick_array_free(ThetaDataDxMarketValueTickArray arr);
void thetadatadx_calendar_day_array_free(ThetaDataDxCalendarDayArray arr);
void thetadatadx_interest_rate_tick_array_free(ThetaDataDxInterestRateTickArray arr);
void thetadatadx_trade_quote_tick_array_free(ThetaDataDxTradeQuoteTickArray arr);
void thetadatadx_option_contract_array_free(ThetaDataDxOptionContractArray arr);
void thetadatadx_string_array_free(ThetaDataDxStringArray arr);
/** Free a subscription array returned by an active-subscriptions query.
 *  @param arr Array from thetadatadx_*_active_subscriptions; no-op when NULL.
 *             Call exactly once. */
void thetadatadx_subscription_array_free(ThetaDataDxSubscriptionArray* arr);

/* ── Arrow IPC terminal for history tick rows ── */

/* Heap-owned byte buffer (Arrow IPC stream) returned by the per-tick
 * thetadatadx_*_to_arrow_ipc terminals. Caller MUST free with thetadatadx_arrow_bytes_free.
 * Layout-identical to ThetaDataDxFlatFileBytes. */
typedef struct ThetaDataDxArrowBytes {
    const uint8_t* data;
    size_t len;
} ThetaDataDxArrowBytes;

/** Serialise a span of history tick rows as an Arrow IPC stream — the same
 *  columnar exit Python exposes via <TickName>List.to_arrow(). The element
 *  type is the layout-pinned tick struct the matching history endpoint
 *  returns.
 *  @param rows Pointer to the tick rows to serialise; may be NULL only when
 *              len is 0 (a valid zero-row stream).
 *  @param len Number of rows referenced by rows.
 *  @return An Arrow IPC byte buffer on success that the caller MUST free with
 *          thetadatadx_arrow_bytes_free, or (data=NULL, len=0) on error with
 *          thetadatadx_last_error() set. */
ThetaDataDxArrowBytes thetadatadx_eod_ticks_to_arrow_ipc(const ThetaDataDxEodTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_ohlc_ticks_to_arrow_ipc(const ThetaDataDxOhlcTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_ticks_to_arrow_ipc(const ThetaDataDxTradeTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_quote_ticks_to_arrow_ipc(const ThetaDataDxQuoteTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_greeks_all_ticks_to_arrow_ipc(const ThetaDataDxGreeksAllTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_greeks_eod_ticks_to_arrow_ipc(const ThetaDataDxGreeksEodTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_greeks_first_order_ticks_to_arrow_ipc(const ThetaDataDxGreeksFirstOrderTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_greeks_second_order_ticks_to_arrow_ipc(const ThetaDataDxGreeksSecondOrderTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_greeks_third_order_ticks_to_arrow_ipc(const ThetaDataDxGreeksThirdOrderTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_all_ticks_to_arrow_ipc(const ThetaDataDxTradeGreeksAllTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_first_order_ticks_to_arrow_ipc(const ThetaDataDxTradeGreeksFirstOrderTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_second_order_ticks_to_arrow_ipc(const ThetaDataDxTradeGreeksSecondOrderTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_third_order_ticks_to_arrow_ipc(const ThetaDataDxTradeGreeksThirdOrderTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_implied_volatility_ticks_to_arrow_ipc(const ThetaDataDxTradeGreeksImpliedVolatilityTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_iv_ticks_to_arrow_ipc(const ThetaDataDxIvTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_price_ticks_to_arrow_ipc(const ThetaDataDxPriceTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_index_price_at_time_ticks_to_arrow_ipc(const ThetaDataDxIndexPriceAtTimeTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_open_interest_ticks_to_arrow_ipc(const ThetaDataDxOpenInterestTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_market_value_ticks_to_arrow_ipc(const ThetaDataDxMarketValueTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_calendar_days_to_arrow_ipc(const ThetaDataDxCalendarDay* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_interest_rate_ticks_to_arrow_ipc(const ThetaDataDxInterestRateTick* rows, size_t len);
ThetaDataDxArrowBytes thetadatadx_trade_quote_ticks_to_arrow_ipc(const ThetaDataDxTradeQuoteTick* rows, size_t len);

/** Free a byte buffer returned by any thetadatadx_*_to_arrow_ipc terminal.
 *  @param bytes Buffer from a thetadatadx_*_to_arrow_ipc call; a (data=NULL, len=0)
 *               buffer is a no-op. Call exactly once. */
void thetadatadx_arrow_bytes_free(ThetaDataDxArrowBytes bytes);

/* ── Column presence + projected Arrow IPC terminal for decode-fed history ── */

/* Heap-owned set of present schema-column names plus optional symbol
 * attribution (the decode's ColumnPresence crossing the C boundary). Built by
 * a thetadatadx_*_present_columns terminal and consumed by the matching
 * thetadatadx_*_to_arrow_ipc_projected terminal. Caller MUST free with
 * thetadatadx_column_presence_free.
 *
 * `symbol` carries a constant root value for responses whose wire has one
 * symbol across every row.
 *
 * `symbols` carries a multi-symbol snapshot's per-row `symbol` (root) values —
 * one NUL-terminated C string per decoded row — so the projected serialiser
 * emits a leading per-row `symbol` column attributing each row to its
 * underlying. It is NULL for every other response and takes precedence over
 * the constant `symbol` field. */
typedef struct ThetaDataDxColumnPresence {
    const char* const* names;
    size_t len;
    const char* symbol;
    const char* const* symbols;
    size_t symbols_len;
} ThetaDataDxColumnPresence;

/** Free a ThetaDataDxColumnPresence returned by any thetadatadx_*_present_columns
 *  terminal, including its names and symbol attribution.
 *  @param presence Carrier from a thetadatadx_*_present_columns call; a
 *                  (names=NULL, len=0) carrier is a no-op. Call exactly once. */
void thetadatadx_column_presence_free(ThetaDataDxColumnPresence presence);

/* Build the wire-column presence set for a response from its header names
 * (the DataTable headers), via the same decode logic the buffered .await path
 * uses. headers may be NULL only when len is 0. The returned carrier MUST be
 * freed with thetadatadx_column_presence_free. */
ThetaDataDxColumnPresence thetadatadx_eod_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_ohlc_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_quote_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_greeks_all_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_greeks_eod_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_greeks_first_order_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_greeks_second_order_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_greeks_third_order_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_greeks_all_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_greeks_first_order_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_greeks_second_order_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_greeks_third_order_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_greeks_implied_volatility_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_iv_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_price_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_index_price_at_time_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_open_interest_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_market_value_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_calendar_days_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_interest_rate_ticks_present_columns(const char* const* headers, size_t len);
ThetaDataDxColumnPresence thetadatadx_trade_quote_ticks_present_columns(const char* const* headers, size_t len);

/* Serialise a span of history tick rows as a PROJECTED Arrow IPC stream —
 * only the columns presence names (from the matching _present_columns), the
 * terminal-exact columnar exit Python's <TickName>List.to_arrow() gives on a
 * decode result. rows may be NULL only when len is 0. The presence carrier is
 * borrowed (still owned by the caller). Returns an Arrow IPC byte buffer the
 * caller MUST free with thetadatadx_arrow_bytes_free, or (data=NULL, len=0) on
 * error with thetadatadx_last_error() set. */
ThetaDataDxArrowBytes thetadatadx_eod_ticks_to_arrow_ipc_projected(const ThetaDataDxEodTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_ohlc_ticks_to_arrow_ipc_projected(const ThetaDataDxOhlcTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_quote_ticks_to_arrow_ipc_projected(const ThetaDataDxQuoteTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_greeks_all_ticks_to_arrow_ipc_projected(const ThetaDataDxGreeksAllTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_greeks_eod_ticks_to_arrow_ipc_projected(const ThetaDataDxGreeksEodTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_greeks_first_order_ticks_to_arrow_ipc_projected(const ThetaDataDxGreeksFirstOrderTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_greeks_second_order_ticks_to_arrow_ipc_projected(const ThetaDataDxGreeksSecondOrderTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_greeks_third_order_ticks_to_arrow_ipc_projected(const ThetaDataDxGreeksThirdOrderTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_all_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeGreeksAllTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_first_order_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeGreeksFirstOrderTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_second_order_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeGreeksSecondOrderTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_third_order_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeGreeksThirdOrderTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_greeks_implied_volatility_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeGreeksImpliedVolatilityTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_iv_ticks_to_arrow_ipc_projected(const ThetaDataDxIvTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_price_ticks_to_arrow_ipc_projected(const ThetaDataDxPriceTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_index_price_at_time_ticks_to_arrow_ipc_projected(const ThetaDataDxIndexPriceAtTimeTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_open_interest_ticks_to_arrow_ipc_projected(const ThetaDataDxOpenInterestTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_market_value_ticks_to_arrow_ipc_projected(const ThetaDataDxMarketValueTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_calendar_days_to_arrow_ipc_projected(const ThetaDataDxCalendarDay* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_interest_rate_ticks_to_arrow_ipc_projected(const ThetaDataDxInterestRateTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);
ThetaDataDxArrowBytes thetadatadx_trade_quote_ticks_to_arrow_ipc_projected(const ThetaDataDxTradeQuoteTick* rows, size_t len, ThetaDataDxColumnPresence presence, const char* symbol);

/* ── Streaming Arrow RecordBatch reader (pull-based) ── */

/** Opaque handle to a live pull-based Arrow RecordBatch reader. Created by
 *  thetadatadx_client_batches_open, drained by
 *  thetadatadx_record_batch_stream_next_ipc, closed by
 *  thetadatadx_record_batch_stream_close, freed by
 *  thetadatadx_record_batch_stream_free.
 *
 *  thetadatadx_record_batch_stream_close is safe to call from another thread
 *  while a thetadatadx_record_batch_stream_next_ipc pull is in flight: the
 *  pull is woken and returns end of stream. thetadatadx_record_batch_stream_free
 *  takes ownership of the opaque handle and must be serialized with all other
 *  entry points for the same handle. */
typedef struct ThetaDataDxRecordBatchStream ThetaDataDxRecordBatchStream;

/** Backpressure: lossless block (applies backpressure to the wire). */
#define THETADATADX_BACKPRESSURE_BLOCK 0
/** Backpressure: bounded buffer, drop the oldest batch on overflow (counted
 *  by thetadatadx_record_batch_stream_dropped). */
#define THETADATADX_BACKPRESSURE_DROP_OLDEST 1

/** Open a pull-based Arrow RecordBatch reader over the unified client's
 *  stream — a sibling to thetadatadx_client_set_callback. Subscribe first on
 *  the same surface, then open. Starts the streaming session.
 *  @param handle Client from thetadatadx_client_connect.
 *  @param batch_size Rows per batch (0 clamped to 1).
 *  @param linger_ms Partial-batch flush deadline in ms (quiet-stream flush).
 *  @param backpressure THETADATADX_BACKPRESSURE_BLOCK or _DROP_OLDEST.
 *  @param capacity Bounded-buffer depth in batches for drop-oldest (ignored
 *                  for block; may be 0).
 *  @return Reader handle, or NULL with thetadatadx_last_error() set on
 *          failure. Free with thetadatadx_record_batch_stream_free. */
ThetaDataDxRecordBatchStream* thetadatadx_client_batches_open(const ThetaDataDxClient* handle,
                                                              size_t batch_size,
                                                              uint64_t linger_ms,
                                                              int32_t backpressure,
                                                              size_t capacity);

/** Block for the next batch and serialise it as an Arrow IPC stream into
 *  *out.
 *  @param stream Reader from thetadatadx_client_batches_open.
 *  @param out Out-param, always initialised: holds the batch IPC bytes on a 0
 *             return (free with thetadatadx_arrow_bytes_free), or empty
 *             (data=NULL, len=0) otherwise.
 *  @return 0 = a batch was produced; 1 = clean end of stream; -1 = error
 *          (thetadatadx_last_error() set). */
int32_t thetadatadx_record_batch_stream_next_ipc(const ThetaDataDxRecordBatchStream* stream,
                                                 ThetaDataDxArrowBytes* out);

/** Serialise the reader's fixed schema as a schema-only Arrow IPC stream into
 *  *out, so a reader can report its schema before the first batch.
 *  @return 0 on success (out holds schema IPC bytes, free with
 *          thetadatadx_arrow_bytes_free); -1 on error (out empty,
 *          thetadatadx_last_error() set). */
int32_t thetadatadx_record_batch_stream_schema_ipc(const ThetaDataDxRecordBatchStream* stream,
                                                   ThetaDataDxArrowBytes* out);

/** Number of batches dropped so far under the drop-oldest policy. Always 0
 *  under block. */
uint64_t thetadatadx_record_batch_stream_dropped(const ThetaDataDxRecordBatchStream* stream);

/** Stop the reader, tear down the streaming session, WITHOUT freeing the handle.
 *  Safe to call from another thread while a pull is in flight: it wakes the
 *  pull (which returns 1, clean end of stream) and shuts the session down.
 *  Idempotent. The handle stays valid and must still be released with
 *  thetadatadx_record_batch_stream_free. A NULL handle is a no-op. */
void thetadatadx_record_batch_stream_close(const ThetaDataDxRecordBatchStream* stream);

/** Release the reader handle. Signals close first, then drops this handle's
 *  reference. This call takes ownership of the opaque handle and must be
 *  serialized with next_ipc, schema_ipc, dropped, and close for the same
 *  handle. To tear down from another thread, call close to wake a parked pull,
 *  wait for in-flight entry points to return, then free. A NULL handle is a
 *  no-op. After this call the handle is invalid. */
void thetadatadx_record_batch_stream_free(ThetaDataDxRecordBatchStream* stream);

/* ── Error ── */

/** Retrieve the last error message for the current thread.
 *  @return The error string, or NULL when no error is set. The pointer is
 *          valid only until the next FFI call on the same thread; do NOT
 *          free it.
 *  @note Thread-local: each thread observes only its own last error. */
const char* thetadatadx_last_error(void);

/** Clear the thread-local error string.
 *  @note Higher-level wrappers should call this before issuing an FFI call
 *        so they can distinguish "the call set a new error" from "the
 *        previous call left a stale error in the slot" when an empty value
 *        (e.g. zero rows) is also a valid success outcome. */
void thetadatadx_clear_error(void);

/** Typed discriminant of the last FFI error on the current thread. Higher-
 *  level bindings (the C++ exception hierarchy below, the typed error
 *  subclasses in the TypeScript SDK) dispatch on this to pick the right
 *  exception / error subclass without substring-matching the formatted
 *  error string. The string from thetadatadx_last_error() carries the diagnostic.
 *  @return One of the THETADATADX_ERR_* discriminants below; THETADATADX_ERR_NONE when no
 *          error is set or after thetadatadx_clear_error(). */
int32_t thetadatadx_last_error_code(void);

/** Server-supplied rate-limit back-off of the last FFI error on the current
 *  thread, in milliseconds. Set only for a rate-limit error whose upstream
 *  status attached the hint. The C++ RateLimitError::retry_after() surfaces
 *  this as a typed value.
 *  @return The back-off in milliseconds, or THETADATADX_RETRY_AFTER_NONE when
 *          the error carries no retry hint (every non-rate-limit error reads
 *          this sentinel). */
int64_t thetadatadx_last_error_retry_after_ms(void);

/* Sentinel returned by `thetadatadx_last_error_retry_after_ms()` when the last
 * error carries no rate-limit back-off hint. */
#define THETADATADX_RETRY_AFTER_NONE (-1)

/* Error-code discriminants returned by `thetadatadx_last_error_code()`. */
#define THETADATADX_ERR_NONE 0
#define THETADATADX_ERR_OTHER 1
#define THETADATADX_ERR_AUTHENTICATION 2
#define THETADATADX_ERR_INVALID_CREDENTIALS 3
#define THETADATADX_ERR_SUBSCRIPTION 4
#define THETADATADX_ERR_RATE_LIMIT 5
#define THETADATADX_ERR_NOT_FOUND 6
#define THETADATADX_ERR_DEADLINE_EXCEEDED 7
#define THETADATADX_ERR_UNAVAILABLE 8
#define THETADATADX_ERR_NETWORK 9
#define THETADATADX_ERR_SCHEMA_MISMATCH 10
#define THETADATADX_ERR_STREAM 11
#define THETADATADX_ERR_CONFIG 12
#define THETADATADX_ERR_INVALID_PARAMETER 13

/* ── Credentials ── */

/** Create a credentials handle from an email and password.
 *  @param email Account email; must be non-NULL.
 *  @param password Account password; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_email(const char* email, const char* password);

/** Create a credentials handle that authenticates with an API key.
 *  @param api_key API key; must be non-NULL. Trimmed and held as secret
 *         material on the handle.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_api_key(const char* api_key);

/** Create a credentials handle that authenticates with an API key paired
 *  with an account email.
 *  @param email Account email; must be non-NULL. An empty email is dropped.
 *  @param api_key API key; must be non-NULL. Trimmed and held as secret
 *         material on the handle.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_api_key_with_email(const char* email, const char* api_key);

/** Create a credentials handle by reading a file (line 1 = email,
 *  line 2 = password).
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_file(const char* path);

/** Source a credentials handle strictly from the THETADATA_API_KEY
 *  environment variable. Strict: an unset or whitespace-only value is an
 *  error rather than a silent fallback, and there is no creds.txt file
 *  fallback. Use thetadatadx_credentials_from_env_or_file when a file
 *  fallback is wanted instead.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_env(void);

/** Source a credentials handle from the environment, falling back to a file.
 *  When THETADATA_API_KEY is set and non-empty an API key is used; otherwise
 *  the two-line file (line 1 = email, line 2 = password) at path is read.
 *  @param path Filesystem path to the fallback credentials file; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_env_or_file(const char* path);

/** Source a credentials handle from a .env-format file.
 *  The file uses the common .env grammar (one KEY=VALUE per line, optional
 *  export prefix, # comment lines, optional matching quotes). When
 *  THETADATA_API_KEY is present and non-empty an API key is used; otherwise a
 *  complete THETADATA_EMAIL + THETADATA_PASSWORD pair builds email + password
 *  credentials.
 *  @param path Filesystem path to the .env file; must be non-NULL.
 *  @return Heap-owned ThetaDataDxCredentials the caller must release with
 *          thetadatadx_credentials_free, or NULL on error (check thetadatadx_last_error()). */
ThetaDataDxCredentials* thetadatadx_credentials_from_dotenv(const char* path);

/** Release a credentials handle.
 *  @param creds Handle from thetadatadx_credentials_from_email /
 *               thetadatadx_credentials_from_file; no-op when NULL. Call exactly once. */
void thetadatadx_credentials_free(ThetaDataDxCredentials* creds);

/* ── Config ── */

/** Create a production config (ThetaData NJ datacenter).
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free. */
ThetaDataDxConfig* thetadatadx_config_production(void);

/** Create a dev streaming config (port 20200, infinite historical replay).
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free. */
ThetaDataDxConfig* thetadatadx_config_dev(void);

/** Create a market-data-staging config (market-data staging cluster + auth marker;
 *  streaming stays on production). Testing, unstable.
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free. */
ThetaDataDxConfig* thetadatadx_config_stage(void);

/** Select the market-data environment on a config handle in place:
 *  kind 0 = production, kind 1 = staging. The market-data and streaming
 *  channels are selected independently, so this leaves the streaming
 *  channel untouched.
 *  @param config Config handle to mutate.
 *  @param kind 0 for PROD, 1 for STAGE.
 *  @return 0 on success, or -1 on error (config is null, or kind is
 *          outside {0, 1}); check thetadatadx_last_error(). */
int32_t thetadatadx_config_with_market_data_environment(ThetaDataDxConfig* config, int32_t kind);

/** Select the streaming environment on a config handle in place:
 *  kind 0 = production, kind 1 = dev. The streaming and market-data
 *  channels are selected independently, so this leaves the market-data
 *  channel and the auth marker untouched.
 *  @param config Config handle to mutate.
 *  @param kind 0 for PROD, 1 for DEV.
 *  @return 0 on success, or -1 on error (config is null, or kind is
 *          outside {0, 1}); check thetadatadx_last_error(). */
int32_t thetadatadx_config_with_streaming_environment(ThetaDataDxConfig* config, int32_t kind);

/** Source a config from a .env-format file. Starts from the production
 *  configuration and applies the cluster keys carried by the file:
 *  THETADATA_MARKET_DATA_TYPE (PROD / STAGE, case-insensitive) selects the
 *  environment, and the optional THETADATA_MARKET_DATA_HOST /
 *  THETADATA_STREAMING_HOST keys override the hosts (an explicit host wins
 *  over the environment default). Reads the same file format and keys as
 *  thetadatadx_credentials_from_dotenv, so one .env can carry both
 *  THETADATA_API_KEY and THETADATA_MARKET_DATA_TYPE.
 *  @param path Path to the .env file.
 *  @return Heap-owned ThetaDataDxConfig the caller must release with
 *          thetadatadx_config_free, or NULL on error
 *          (check thetadatadx_last_error()). */
ThetaDataDxConfig* thetadatadx_config_from_dotenv(const char* path);

/** Release a config handle.
 *  @param config Handle from a config factory; no-op when NULL.
 *                Call exactly once. */
void thetadatadx_config_free(ThetaDataDxConfig* config);

/**
 * Set the streaming reconnect policy on a config handle.
 *   policy=0: Auto (default) -- auto-reconnect with split per-class attempt
 *             budgets. Generic transient failures (TimedOut, ServerRestarting,
 *             Unspecified) use the budget set by
 *             `thetadatadx_config_set_reconnect_max_attempts`; the rate-limited
 *             (`TooManyRequests`) class uses
 *             `thetadatadx_config_set_reconnect_max_rate_limited_attempts`. Counters
 *             reset after a continuous data-flow window configured via
 *             `thetadatadx_config_set_reconnect_stable_window_secs`.
 *   policy=1: Manual -- no auto-reconnect.
 * @param config Config handle to mutate.
 * @param policy Reconnect policy selector (0 = Auto, 1 = Manual).
 * @return 0 on success, -1 on an invalid policy (outside {0, 1}) or null
 *         config. A rejected policy sets thetadatadx_last_error_code to
 *         THETADATADX_ERR_INVALID_PARAMETER so an unknown value is rejected with the
 *         same typed class the Python / TypeScript bindings raise, never
 *         silently coerced to Auto.
 */
int32_t thetadatadx_config_set_reconnect_policy(ThetaDataDxConfig* config, int policy);

/**
 * Set the per-class transient-failure attempt budget for the
 * auto-reconnect path. Default 30. No effect unless the reconnect
 * policy is Auto.
 * @param config Config handle to mutate; no-op when NULL.
 * @param max_attempts Per-class transient-failure attempt budget.
 */
void thetadatadx_config_set_reconnect_max_attempts(ThetaDataDxConfig* config,
                                           uint32_t max_attempts);

/**
 * Set the per-class rate-limited (`TooManyRequests`) attempt budget for
 * the auto-reconnect path. Default 100. No effect unless the reconnect
 * policy is Auto.
 * @param config Config handle to mutate; no-op when NULL.
 * @param max_rate_limited_attempts Per-class rate-limited attempt budget.
 */
void thetadatadx_config_set_reconnect_max_rate_limited_attempts(
    ThetaDataDxConfig* config, uint32_t max_rate_limited_attempts);

/**
 * Set the continuous successful-data-flow window (in seconds) after
 * which the auto-reconnect attempt counters reset. Default 60. No
 * effect unless the reconnect policy is Auto.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Stable-window length in seconds.
 */
void thetadatadx_config_set_reconnect_stable_window_secs(ThetaDataDxConfig* config,
                                                 uint64_t secs);

/**
 * Set the reconnect delay (ms) honoured for generic transient
 * disconnects (TimedOut, ServerRestarting, Unspecified, ...). Applied
 * to the streaming session at connect time. Default 250.
 * @param config Config handle to mutate; no-op when NULL.
 * @param ms Reconnect delay in milliseconds.
 */
void thetadatadx_config_set_reconnect_wait_ms(ThetaDataDxConfig* config, uint64_t ms);

/**
 * Read the current reconnect wait_ms setting.
 * @param config Config handle to read.
 * @param out_ms Receives the configured millisecond delay on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_wait_ms(const ThetaDataDxConfig* config, uint64_t* out_ms);

/**
 * Set the reconnect delay (ms) honoured for `TooManyRequests`
 * rate-limited disconnects. Default 130_000 (the upstream-instructed
 * 130 s rate-limit cooldown).
 * @param config Config handle to mutate; no-op when NULL.
 * @param ms Reconnect delay in milliseconds.
 */
void thetadatadx_config_set_reconnect_wait_rate_limited_ms(ThetaDataDxConfig* config, uint64_t ms);

/**
 * Read the current reconnect wait_rate_limited_ms setting.
 * @param config Config handle to read.
 * @param out_ms Receives the configured millisecond delay on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_wait_rate_limited_ms(const ThetaDataDxConfig* config, uint64_t* out_ms);


/**
 * Read the configured reconnect policy selector.
 * @param config Config handle to read.
 * @param out_policy Receives 0 (Auto), 1 (Manual), or 2 (Custom) on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_policy(const ThetaDataDxConfig* config, int32_t* out_policy);

/**
 * Read the generic-transient reconnect attempt budget (default 30).
 * When the policy is not Auto, writes the default-limits value.
 * @param config Config handle to read.
 * @param out Receives the attempt budget on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_max_attempts(const ThetaDataDxConfig* config, uint32_t* out);

/**
 * Read the rate-limited reconnect attempt budget (default 100).
 * @param config Config handle to read.
 * @param out Receives the attempt budget on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_max_rate_limited_attempts(const ThetaDataDxConfig* config,
                                                           uint32_t* out);

/**
 * Set the ServerRestarting reconnect attempt budget. Default 60. No
 * effect unless the reconnect policy is Auto.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n ServerRestarting attempt budget.
 */
void thetadatadx_config_set_reconnect_max_server_restart_attempts(ThetaDataDxConfig* config, uint32_t n);

/**
 * Read the ServerRestarting reconnect attempt budget (default 60).
 * @param config Config handle to read.
 * @param out Receives the attempt budget on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_max_server_restart_attempts(const ThetaDataDxConfig* config,
                                                             uint32_t* out);

/**
 * Read the stable-window reset interval in seconds (default 60).
 * @param config Config handle to read.
 * @param out Receives the stable-window length in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_stable_window_secs(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the wall-clock reconnect envelope (seconds) for the
 * generic-transient and server-restart classes, measured from the
 * first attempt of a consecutive-reconnect sequence. 0 disables the
 * envelope (attempt budgets only). Default 300. No effect unless the
 * reconnect policy is Auto.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Reconnect envelope in seconds (0 disables).
 */
void thetadatadx_config_set_reconnect_max_elapsed_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the wall-clock reconnect envelope in seconds (default 300;
 * 0 = disabled).
 * @param config Config handle to read.
 * @param out Receives the envelope in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_max_elapsed_secs(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the cap (ms) on the exponential generic-transient reconnect
 * ladder. The ladder starts at reconnect_wait_ms and doubles per
 * consecutive attempt up to this value. Default 30_000.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Ladder cap in milliseconds.
 */
void thetadatadx_config_set_reconnect_wait_max_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current reconnect wait_max_ms setting (default 30_000).
 * @param config Config handle to read.
 * @param out Receives the ladder cap in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_wait_max_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the flat reconnect cadence (ms) for ServerRestarting
 * disconnects. Default 5_000.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Reconnect cadence in milliseconds.
 */
void thetadatadx_config_set_reconnect_wait_server_restart_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current reconnect wait_server_restart_ms setting (default 5_000).
 * @param config Config handle to read.
 * @param out Receives the cadence in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_wait_server_restart_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the jitter strategy applied to every reconnect delay.
 *   mode=0: Full (default) -- sample uniformly from [0, delay].
 *   mode=1: Equal -- delay/2 + uniform(0, delay/2).
 *   mode=2: Decorrelated -- walk relative to the previous delay.
 *   mode=3: None -- deterministic delays (tests only).
 * @param config Config handle to mutate.
 * @param mode Jitter strategy selector (0-3 per the list above).
 * @return 0 on success, -1 on an invalid mode or null config.
 */
int32_t thetadatadx_config_set_reconnect_jitter(ThetaDataDxConfig* config, int32_t mode);

/**
 * Read the configured reconnect jitter mode. Same encoding as
 * thetadatadx_config_set_reconnect_jitter.
 * @param config Config handle to read.
 * @param out_mode Receives the jitter mode on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_jitter(const ThetaDataDxConfig* config, int32_t* out_mode);

/**
 * Set the subscription-replay burst size used after an auto-reconnect:
 * frames are written in bursts of this many, each burst flushed and
 * followed by a jittered replay_pace_ms pause. Minimum 1 (validated at
 * connect). Default 50.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n Replay burst size in frames (minimum 1).
 */
void thetadatadx_config_set_reconnect_replay_burst_size(ThetaDataDxConfig* config, uint32_t n);

/**
 * Read the current replay_burst_size setting (default 50).
 * @param config Config handle to read.
 * @param out Receives the burst size on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_replay_burst_size(const ThetaDataDxConfig* config, uint32_t* out);

/**
 * Set the pause (ms) between subscription-replay bursts after an
 * auto-reconnect. 0 removes the pause. Default 5.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Inter-burst pause in milliseconds (0 disables).
 */
void thetadatadx_config_set_reconnect_replay_pace_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current replay_pace_ms setting (default 5).
 * @param config Config handle to read.
 * @param out Receives the pause in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_reconnect_replay_pace_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Reconnect-decision callback for thetadatadx_config_set_reconnect_callback.
 * Invoked on the streaming I/O thread after each retriable involuntary
 * disconnect.
 * @param reason The disconnect-reason discriminant.
 * @param attempt The 1-based consecutive-reconnect counter.
 * @param user_data The opaque pointer registered alongside the callback.
 * @return The reconnect delay in milliseconds, or any negative value to
 *         stop reconnecting.
 * @note This callback must not unwind across the C ABI. A C++ throw or a C
 *       longjmp that escapes the callback into the calling frame is undefined
 *       behavior, the same as for any C library. The library wraps each
 *       invocation to contain a fault on its own side of the boundary, but
 *       that does not contain an exception thrown out of your callback. Catch
 *       and handle every exception before returning a decision.
 */
typedef int64_t (*ThetaDataDxReconnectCallback)(int32_t reason, uint32_t attempt, void* user_data);

/**
 * Install a custom reconnect policy driven by a C callback. Permanent
 * disconnect reasons never reach the callback.
 * @param config Config handle to mutate.
 * @param cb The reconnect-decision callback; NULL restores the default Auto
 *           policy.
 * @param user_data Opaque pointer passed back to cb unchanged.
 * @return 0 on success, -1 if config is null.
 * @note cb runs on the streaming I/O thread: cb and user_data must be safe
 *       to use from another thread for as long as any client built from
 *       this config is alive.
 */
int32_t thetadatadx_config_set_reconnect_callback(ThetaDataDxConfig* config, ThetaDataDxReconnectCallback cb,
                                          void* user_data);

/**
 * Set the streaming read timeout (ms): the no-frames deadline after which
 * the streaming session is declared dead and reconnects. Default 10_000;
 * validated to [100, 60_000] at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Read timeout in milliseconds.
 */
void thetadatadx_config_set_streaming_timeout_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current streaming timeout_ms setting (default 10_000).
 * @param config Config handle to read.
 * @param out Receives the timeout in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_timeout_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the per-server connect timeout (ms) for the streaming
 * connection. Default 2_000; validated to [1_000, 60_000] at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Connect timeout in milliseconds.
 */
void thetadatadx_config_set_streaming_connect_timeout_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current streaming connect_timeout_ms setting (default 2_000).
 * @param config Config handle to read.
 * @param out Receives the timeout in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_connect_timeout_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the streaming heartbeat ping interval (ms). Default 250; validated to
 * [100, 300_000] at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Ping interval in milliseconds.
 */
void thetadatadx_config_set_streaming_ping_interval_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current streaming ping_interval_ms setting (default 250).
 * @param config Config handle to read.
 * @param out Receives the interval in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_ping_interval_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the per-iteration blocking-read slice (ms) for the streaming
 * session. Default 25; validated to [10, 500] at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Read slice in milliseconds.
 */
void thetadatadx_config_set_streaming_io_read_slice_ms(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current streaming io_read_slice_ms setting (default 25).
 * @param config Config handle to read.
 * @param out Receives the read slice in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_io_read_slice_ms(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the TCP keepalive idle time (seconds) before the first kernel
 * probe on a silent streaming socket. Default 5; validated to [1, 7_200]
 * at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Keepalive idle time in seconds.
 */
void thetadatadx_config_set_streaming_keepalive_idle_secs(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current streaming keepalive_idle_secs setting (default 5).
 * @param config Config handle to read.
 * @param out Receives the idle time in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_keepalive_idle_secs(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the interval (seconds) between TCP keepalive probes. Default 2;
 * validated to [1, 75] at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Keepalive probe interval in seconds.
 */
void thetadatadx_config_set_streaming_keepalive_interval_secs(ThetaDataDxConfig* config, uint64_t v);

/**
 * Read the current streaming keepalive_interval_secs setting (default 2).
 * @param config Config handle to read.
 * @param out Receives the probe interval in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_keepalive_interval_secs(const ThetaDataDxConfig* config, uint64_t* out);

/**
 * Set the number of unanswered TCP keepalive probes after which the
 * kernel declares the streaming connection dead (where the platform exposes
 * the knob). Default 2; validated to [1, 10] at connect.
 * @param config Config handle to mutate; no-op when NULL.
 * @param v Keepalive probe-failure count.
 */
void thetadatadx_config_set_streaming_keepalive_retries(ThetaDataDxConfig* config, uint32_t v);

/**
 * Read the current streaming keepalive_retries setting (default 2).
 * @param config Config handle to read.
 * @param out Receives the probe-failure count on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_keepalive_retries(const ThetaDataDxConfig* config, uint32_t* out);

/**
 * Set the streaming event ring buffer size (slots). Must be a power of two
 * >= 64; invalid values are rejected at the setter (thetadatadx_last_error).
 * Default 131_072.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n Ring buffer size in slots (power of two, >= 64).
 */
void thetadatadx_config_set_streaming_ring_size(ThetaDataDxConfig* config, size_t n);

/**
 * Read the current streaming ring_size setting (default 131_072).
 * @param config Config handle to read.
 * @param out Receives the ring buffer size in slots on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_ring_size(const ThetaDataDxConfig* config, size_t* out);

/**
 * Set the streaming host-selection policy.
 *   policy=0: Shuffled (default) -- fault-domain-aware per-client
 *             shuffle; a fleet spreads across hosts and consecutive
 *             failover attempts cross physical machines.
 *   policy=1: FixedOrder -- use the declared host order verbatim.
 * @param config Config handle to mutate.
 * @param policy Host-selection policy selector (0 = Shuffled, 1 = FixedOrder).
 * @return 0 on success, -1 on an invalid policy or null config.
 */
int32_t thetadatadx_config_set_streaming_host_selection(ThetaDataDxConfig* config, int32_t policy);

/**
 * Read the configured streaming host-selection policy. Same encoding as
 * thetadatadx_config_set_streaming_host_selection.
 * @param config Config handle to read.
 * @param out_policy Receives the host-selection policy on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_streaming_host_selection(const ThetaDataDxConfig* config, int32_t* out_policy);

/**
 * Set the streaming host-shuffle seed using the (has_value, seed) widened
 * shape. Ignored under the FixedOrder policy.
 * @param config Config handle to mutate.
 * @param has_value false (default) derives a fresh per-client seed so a
 *                  fleet shuffles independently; true makes the shuffled
 *                  order deterministic, useful for fleet sharding and tests.
 * @param seed The deterministic seed, honoured only when has_value is true.
 * @return 0 on success, -1 if config is null.
 */
int32_t thetadatadx_config_set_streaming_host_shuffle_seed(ThetaDataDxConfig* config, bool has_value, uint64_t seed);

/**
 * Read the current streaming host-shuffle seed.
 * @param config Config handle to read.
 * @param out_has_value Receives false for the per-client-entropy sentinel,
 *                      true when an explicit seed is set.
 * @param out_seed Receives the seed when out_has_value is true.
 * @return 0 on success, -1 if any pointer is null.
 */
int32_t thetadatadx_config_get_streaming_host_shuffle_seed(const ThetaDataDxConfig* config, bool* out_has_value,
                                              uint64_t* out_seed);

/**
 * Set the wall-clock envelope (seconds) for one market-data-channel
 * retry sequence, measured from the first attempt. 0 disables the
 * envelope (attempt budget only). Default 300.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Retry envelope in seconds (0 disables).
 */
void thetadatadx_config_set_retry_max_elapsed_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the current retry max_elapsed value in seconds (default 300; 0 = disabled).
 * @param config Config handle to read.
 * @param out_secs Receives the envelope in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_retry_max_elapsed_secs(const ThetaDataDxConfig* config, uint64_t* out_secs);

/**
 * Toggle AWS-style full jitter on the flatfile retry ladder. Default
 * true; false gives the deterministic schedule, useful for tests.
 * @param config Config handle to mutate; no-op when NULL.
 * @param jitter true enables full jitter, false uses the deterministic schedule.
 */
void thetadatadx_config_set_flatfiles_jitter(ThetaDataDxConfig* config, bool jitter);

/**
 * Read the current flatfiles jitter setting (default true).
 * @param config Config handle to read.
 * @param out_jitter Receives the jitter flag on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flatfiles_jitter(const ThetaDataDxConfig* config, bool* out_jitter);

/**
 * Set the async worker-thread count for embedded runtimes, using the
 * widened (has_value, n) shape so an unset value is distinct from any
 * explicit count across the Python / TypeScript / C++ bindings.
 *
 * The async worker pool is process-global: it is built once, from the
 * config of the first client connected in the process. This setting is
 * therefore honoured when the first client in the process is created;
 * clients connected later share the already-built pool, so setting it on
 * a subsequent config has no effect.
 * @param config Config handle to mutate.
 * @param has_value false leaves the count unset (auto-sized) and ignores n;
 *                  true sets an explicit count. An explicit 0 is preserved
 *                  across the boundary; the connection clamps it to 1.
 * @param n The explicit worker-thread count, honoured only when has_value
 *          is true.
 * @return 0 on success, -1 if config is NULL.
 */
int32_t thetadatadx_config_set_worker_threads(ThetaDataDxConfig* config, bool has_value, size_t n);

/**
 * Read the current async worker-thread count, using the same widened
 * (has_value, n) shape.
 * @param config Config handle to read.
 * @param out_has_value Receives false when the count is unset (auto-sized),
 *                      true when an explicit count is set.
 * @param out_n Receives the count (0 when unset, the explicit count otherwise).
 * @return 0 on success, -1 if any pointer is null.
 */
int32_t thetadatadx_config_get_worker_threads(const ThetaDataDxConfig* config, bool* out_has_value, size_t* out_n);

/* ── RetryPolicy field setters/getters ── */

/**
 * Set the initial backoff delay (ms) for the market-data-channel retry policy.
 * Default 250. Subsequent retries double from here, capped at
 * thetadatadx_config_set_retry_max_delay_ms.
 * @param config Config handle to mutate; no-op when NULL.
 * @param ms Initial backoff delay in milliseconds.
 */
void thetadatadx_config_set_retry_initial_delay_ms(ThetaDataDxConfig* config, uint64_t ms);

/**
 * Read the market-data-channel retry initial-delay setting (ms).
 * @param config Config handle to read.
 * @param out_ms Receives the initial delay in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_retry_initial_delay_ms(const ThetaDataDxConfig* config, uint64_t* out_ms);

/**
 * Set the upper-bound backoff delay (ms) for the market-data-channel retry policy.
 * Default 30_000 (30 s).
 * @param config Config handle to mutate; no-op when NULL.
 * @param ms Upper-bound backoff delay in milliseconds.
 */
void thetadatadx_config_set_retry_max_delay_ms(ThetaDataDxConfig* config, uint64_t ms);

/**
 * Read the market-data-channel retry max-delay setting (ms).
 * @param config Config handle to read.
 * @param out_ms Receives the max delay in milliseconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_retry_max_delay_ms(const ThetaDataDxConfig* config, uint64_t* out_ms);

/**
 * Set the total attempt budget for the market-data-channel retry policy. 1 disables
 * retry (single call only); higher values permit retries up to
 * max_attempts - 1 after the initial call. Default 20.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n Total attempt budget.
 */
void thetadatadx_config_set_retry_max_attempts(ThetaDataDxConfig* config, uint32_t n);

/**
 * Read the market-data-channel retry max-attempts setting.
 * @param config Config handle to read.
 * @param out_n Receives the attempt budget on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_retry_max_attempts(const ThetaDataDxConfig* config, uint32_t* out_n);

/**
 * Toggle AWS-style full-jitter on the market-data-channel retry policy. Default
 * true. false gives the deterministic backoff schedule
 * min(max_delay, initial * 2^attempt), useful for tests.
 * @param config Config handle to mutate; no-op when NULL.
 * @param jitter true enables full jitter, false uses the deterministic schedule.
 */
void thetadatadx_config_set_retry_jitter(ThetaDataDxConfig* config, bool jitter);

/**
 * Read the market-data-channel retry jitter setting.
 * @param config Config handle to read.
 * @param out_jitter Receives the jitter flag on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_retry_jitter(const ThetaDataDxConfig* config, bool* out_jitter);

/* ── FlatFilesConfig field setters/getters ── */

/**
 * Set the total attempt budget for the flatfile retry loop.
 * 1 disables retry (single call only); higher values permit retries
 * up to max_attempts - 1 after the initial call. Default 10.
 * Validated to the range [1, 100] at connect time.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n Total attempt budget.
 */
void thetadatadx_config_set_flatfiles_max_attempts(ThetaDataDxConfig* config, uint32_t n);

/**
 * Read the flatfile retry max-attempts setting.
 * @param config Config handle to read.
 * @param out_n Receives the attempt budget on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flatfiles_max_attempts(const ThetaDataDxConfig* config, uint32_t* out_n);

/**
 * Set the initial backoff delay (seconds) for the flatfile retry loop.
 * Doubles per attempt up to max_backoff_secs. Default 1.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Initial backoff delay in seconds.
 */
void thetadatadx_config_set_flatfiles_initial_backoff_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the flatfile retry initial-backoff setting (seconds).
 * @param config Config handle to read.
 * @param out_secs Receives the initial backoff in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flatfiles_initial_backoff_secs(const ThetaDataDxConfig* config, uint64_t* out_secs);

/**
 * Set the upper-bound backoff delay (seconds) for the flatfile retry
 * loop. The doubling schedule never exceeds this value regardless of
 * attempt number. Default 30. Must be >= initial_backoff (rejected at
 * connect-time validation otherwise).
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Upper-bound backoff delay in seconds.
 */
void thetadatadx_config_set_flatfiles_max_backoff_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the flatfile retry max-backoff setting (seconds).
 * @param config Config handle to read.
 * @param out_secs Receives the max backoff in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flatfiles_max_backoff_secs(const ThetaDataDxConfig* config, uint64_t* out_secs);

/**
 * Set the TCP + TLS connect timeout (seconds) for one flatfile-host
 * attempt. Bounds the connect/auth handshake before the attempt is
 * abandoned and the next host (or the retry ladder) takes over.
 * Default 10.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Connect timeout in seconds.
 */
void thetadatadx_config_set_flatfiles_connect_timeout_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the flatfile connect-timeout setting (seconds).
 * @param config Config handle to read.
 * @param out_secs Receives the connect timeout in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flatfiles_connect_timeout_secs(const ThetaDataDxConfig* config, uint64_t* out_secs);

/**
 * Set the read timeout (seconds) for a single flatfile response frame.
 * Bounds the wait for the next chunk once streaming has begun so a
 * mid-stream stall fails over instead of blocking forever. Default 60.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Read timeout in seconds.
 */
void thetadatadx_config_set_flatfiles_read_timeout_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the flatfile read-timeout setting (seconds).
 * @param config Config handle to read.
 * @param out_secs Receives the read timeout in seconds on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flatfiles_read_timeout_secs(const ThetaDataDxConfig* config, uint64_t* out_secs);

/* ── AuthConfig field setters/getters ── */

/**
 * Set the Nexus auth URL on a config handle.
 * @param config Config handle to mutate.
 * @param url Non-null, NUL-terminated, valid-UTF-8 C string.
 * @return 0 on success, -1 if config is null or url is null / not valid
 *         UTF-8 (check thetadatadx_last_error()).
 */
int32_t thetadatadx_config_set_nexus_url(ThetaDataDxConfig* config, const char* url);

/**
 * Read the configured Nexus auth URL.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST release with
 *         thetadatadx_string_free, or NULL on a null handle / interior-NUL value
 *         (check thetadatadx_last_error()).
 */
char* thetadatadx_config_get_nexus_url(const ThetaDataDxConfig* config);

/**
 * Set the client_type query identifier on a config handle.
 * @param config Config handle to mutate.
 * @param client_type Non-null, NUL-terminated, valid-UTF-8 C string.
 * @return 0 on success, -1 if config is null or client_type is null / not
 *         valid UTF-8 (check thetadatadx_last_error()).
 */
int32_t thetadatadx_config_set_client_type(ThetaDataDxConfig* config, const char* client_type);

/**
 * Read the configured client_type query identifier.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST release with
 *         thetadatadx_string_free, or NULL on a null handle / interior-NUL value
 *         (check thetadatadx_last_error()).
 */
char* thetadatadx_config_get_client_type(const ThetaDataDxConfig* config);

/* ── MetricsConfig field setter/getter ── */

/**
 * Set the Prometheus exporter port on a config handle, using the widened
 * (has_value, port) shape.
 * @param config Config handle to mutate.
 * @param has_value false leaves the exporter disabled and ignores port;
 *                  true enables it. When enabled and the metrics-prometheus
 *                  feature is compiled in, the exporter binds an HTTP
 *                  listener on 0.0.0.0:<port>.
 * @param port The exporter port, honoured only when has_value is true.
 * @return 0 on success, -1 if config is null.
 */
int32_t thetadatadx_config_set_metrics_port(ThetaDataDxConfig* config, bool has_value, uint16_t port);

/**
 * Read the configured Prometheus exporter port, using the same widened
 * (has_value, port) shape.
 * @param config Config handle to read.
 * @param out_has_value Receives false when the exporter is disabled, true
 *                      when a port is set.
 * @param out_port Receives the port (0 when disabled, the set port otherwise).
 * @return 0 on success, -1 if any pointer is null.
 */
int32_t thetadatadx_config_get_metrics_port(const ThetaDataDxConfig* config, bool* out_has_value, uint16_t* out_port);

/**
 * Set streaming flush mode on a config handle.
 *   mode=0: Batched (default) -- flush only on PING every 100ms.
 *   mode=1: Immediate -- flush after every frame write.
 * @param config Config handle to mutate.
 * @param mode Flush mode selector (0 = Batched, 1 = Immediate).
 * @return 0 on success. -1 with thetadatadx_last_error set and thetadatadx_last_error_code =
 *         THETADATADX_ERR_CONFIG when mode is outside {0, 1} or config is null.
 */
int thetadatadx_config_set_flush_mode(ThetaDataDxConfig* config, int mode);

/**
 * Read the current streaming flush mode. Same encoding as
 * thetadatadx_config_set_flush_mode.
 * @param config Config handle to read.
 * @param out_mode Receives 0 (Batched) or 1 (Immediate) on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_flush_mode(const ThetaDataDxConfig* config, int32_t* out_mode);

/**
 * Read the market-data environment carried by the config: "PROD"
 * for the production cluster or "STAGE" for staging. The market-data and
 * streaming environments are selected independently; the production /
 * stage / dev presets (and the THETADATA_MARKET_DATA_TYPE dotenv key) set the
 * market-data channel, and this is the readback of that selection.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST free with
 *         thetadatadx_string_free, or NULL if config is null.
 */
char* thetadatadx_config_get_market_data_environment(const ThetaDataDxConfig* config);

/**
 * Read the streaming environment carried by the config: "PROD" for
 * the production cluster or "DEV" for the dev cluster. The streaming and
 * market-data environments are selected independently; the production /
 * stage / dev presets (and the THETADATA_STREAMING_TYPE dotenv key) set the
 * streaming channel, and this is the readback of that selection.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST free with
 *         thetadatadx_string_free, or NULL if config is null.
 */
char* thetadatadx_config_get_streaming_environment(const ThetaDataDxConfig* config);

/* Sentinel for thetadatadx_config_set_consumer_cpu /
 * _get_consumer_cpu: a negative core id means "unpinned" (the default,
 * OS scheduler). */
#define THETADATADX_CONSUMER_CPU_UNPINNED (-1)

/**
 * Pin the streaming consumer thread to a CPU core.
 *   core >= 0: pin the tick-consumer thread to that core (deterministic,
 *              low-jitter delivery; out-of-range/offline core is a no-op).
 *   core <  0: unpinned (THETADATADX_CONSUMER_CPU_UNPINNED, the default).
 * @param config Config handle to mutate.
 * @param core Core id, or a negative value for unpinned.
 * @return 0 on success, -1 (THETADATADX_ERR_CONFIG) on a null handle.
 */
int32_t thetadatadx_config_set_consumer_cpu(ThetaDataDxConfig* config, int64_t core);

/**
 * Read the streaming consumer-thread CPU pin.
 * @param config Config handle to read.
 * @param out_core Receives the pinned core, or
 *        THETADATADX_CONSUMER_CPU_UNPINNED (-1) when unpinned.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_consumer_cpu(const ThetaDataDxConfig* config, int64_t* out_core);

/* ── Decode pool sizing ── */

/**
 * Set the market-data gRPC host.
 * @param config Config handle to mutate.
 * @param host Non-null, NUL-terminated, valid-UTF-8 C string.
 * @return 0 on success, -1 if config is null or host is null / not valid
 *         UTF-8.
 */
int32_t thetadatadx_config_set_market_data_host(ThetaDataDxConfig* config, const char* host);

/**
 * Read the configured market-data gRPC host.
 * @param config Config handle to read.
 * @return A heap-owned NUL-terminated C string the caller MUST free with
 *         thetadatadx_string_free, or NULL if config is null or the value contains
 *         an interior NUL.
 */
char* thetadatadx_config_get_market_data_host(const ThetaDataDxConfig* config);

/**
 * Set the market-data gRPC port.
 * @param config Config handle to mutate; no-op when NULL.
 * @param port The gRPC port.
 */
void thetadatadx_config_set_market_data_port(ThetaDataDxConfig* config, uint16_t port);

/**
 * Read the configured market-data gRPC port.
 * @param config Config handle to read.
 * @param out_port Receives the gRPC port on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_market_data_port(const ThetaDataDxConfig* config, uint16_t* out_port);

/**
 * Set the warn_on_buffered_threshold_bytes ceiling on a config.
 *
 * Buffered (non-streaming) endpoints log a warning when a response's
 * decoded total exceeds this threshold, guiding users to the streaming
 * variant. The payload is still delivered.
 * @param config Config handle to mutate; no-op when NULL.
 * @param n Threshold in bytes; 0 disables the warning. Default
 *          100 * 1024 * 1024 (100 MiB).
 */
void thetadatadx_config_set_warn_on_buffered_threshold_bytes(ThetaDataDxConfig* config, size_t n);

/**
 * Read the current warn_on_buffered_threshold_bytes setting.
 * @param config Config handle to read.
 * @param out_n Receives the configured byte count on success.
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_warn_on_buffered_threshold_bytes(const ThetaDataDxConfig* config, size_t* out_n);

/**
 * Set the default per-request deadline (seconds) for market-data queries.
 *
 * Bounds every request that did not set its own deadline, so a
 * live-but-silent stream resolves to a timeout instead of blocking
 * forever.
 * @param config Config handle to mutate; no-op when NULL.
 * @param secs Deadline in seconds; 0 no longer disables the default, it is floored to the 300-second default at request time. Default 300.
 */
void thetadatadx_config_set_request_timeout_secs(ThetaDataDxConfig* config, uint64_t secs);

/**
 * Read the current market-data request_timeout_secs setting.
 * @param config Config handle to read.
 * @param out Receives the configured seconds on success (a stored 0 is floored to the 300-second default at request time).
 * @return 0 on success, -1 if either pointer is null.
 */
int32_t thetadatadx_config_get_request_timeout_secs(const ThetaDataDxConfig* config, uint64_t* out);

/* ── MarketDataClient ── */

/** Connect a market-data client to ThetaData servers.
 *  @param creds Credentials handle; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A connected client the caller must release with
 *          thetadatadx_market_data_free, or NULL on connection/auth failure
 *          (check thetadatadx_last_error()). */
ThetaDataDxMarketDataClient* thetadatadx_market_data_connect(const ThetaDataDxCredentials* creds, const ThetaDataDxConfig* config);

/** Connect a market-data client, reading credentials from a file
 *  (line 1 = email, line 2 = password). One-call equivalent of
 *  thetadatadx_credentials_from_file + thetadatadx_market_data_connect.
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A connected client the caller must release with
 *          thetadatadx_market_data_free, or NULL on argument or connection/auth
 *          failure (check thetadatadx_last_error()). */
ThetaDataDxMarketDataClient* thetadatadx_market_data_connect_from_file(const char* path, const ThetaDataDxConfig* config);

/** Release a market-data client handle.
 *  @param client Handle from a thetadatadx_market_data_connect* call; no-op when
 *                NULL. Call exactly once. */
void thetadatadx_market_data_free(ThetaDataDxMarketDataClient* client);

/* ── String free ── */

/** Free a string returned by any thetadatadx_* function.
 *  @param s Heap-owned string from a thetadatadx_* call; no-op when NULL. Call
 *           exactly once. */
void thetadatadx_string_free(char* s);

/* Generated option-aware endpoint declarations. */
#include "endpoint_with_options.h.inc"

/** User callback signature for the thetadatadx_<endpoint>_stream server-stream entry
 *  points. Invoked once per decoded chunk drained from a market-data result.
 *
 *  `rows` points at the first element of a contiguous run of `len` tick
 *  structs -- the SAME layout the matching thetadatadx_<endpoint>_with_options array
 *  returns (e.g. a thetadatadx_option_history_trade_stream chunk is `len` x
 *  ThetaDataDxTradeTick). Cast `rows` to that tick pointer type before indexing. The
 *  pointer is valid only for the duration of the call -- copy any rows the
 *  caller wants to outlive the callback. An empty result drains as zero
 *  invocations (a null `rows` with `len == 0` is never delivered).
 *
 *  `ctx` is the opaque pointer registered alongside the callback; it is passed
 *  back unchanged on every invocation.
 *
 *  This callback must not unwind across the C ABI. A C++ throw or a C longjmp
 *  that escapes the callback into the calling frame is undefined behavior, the
 *  same as for any C library. The library wraps each invocation to contain a
 *  fault on its own side of the boundary, but that does not contain an
 *  exception thrown out of your callback. Catch and handle every exception
 *  before returning. */
typedef void (*ThetaDataDxTickChunkCallback)(const void* rows, size_t len, void* ctx);

/* Generated server-stream endpoint declarations. */
#include "market_data_stream.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Cross-language utility helpers — conditions / exchange / sequences   */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All `thetadatadx_*_name` / `thetadatadx_*_description` / `thetadatadx_exchange_*` returns are
 * NUL-terminated UTF-8 C strings owned by the library — DO NOT FREE. The
 * pointer remains valid for the lifetime of the process. Unknown codes
 * return either "UNKNOWN" (name lookup) or "" (description lookup), never
 * NULL. */

/** Trade condition name lookup.
 *  @param code The trade condition code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_condition_name(int32_t code);

/** Trade condition description lookup.
 *  @param code The trade condition code.
 *  @return A process-lifetime string; "" for unrecognised codes. Never
 *          NULL; DO NOT FREE. */
const char* thetadatadx_condition_description(int32_t code);

/** Whether a trade condition code represents a cancellation.
 *  @param code The trade condition code.
 *  @return true if the code represents a cancellation, false otherwise. */
bool thetadatadx_condition_is_cancel(int32_t code);

/** Whether a trade condition code updates the volume bar.
 *  @param code The trade condition code.
 *  @return true if the code updates the volume bar, false otherwise. */
bool thetadatadx_condition_updates_volume(int32_t code);

/** Quote condition name lookup.
 *  @param code The quote condition code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_quote_condition_name(int32_t code);

/** Quote condition description lookup.
 *  @param code The quote condition code.
 *  @return A process-lifetime string; "" for unrecognised codes. Never
 *          NULL; DO NOT FREE. */
const char* thetadatadx_quote_condition_description(int32_t code);

/** Whether a quote condition is firm (binding).
 *  @param code The quote condition code.
 *  @return true if the quote condition is firm, false otherwise. */
bool thetadatadx_quote_condition_is_firm(int32_t code);

/** Whether a quote condition indicates a trading halt.
 *  @param code The quote condition code.
 *  @return true if the quote condition indicates a trading halt, false
 *          otherwise. */
bool thetadatadx_quote_condition_is_halted(int32_t code);

/** Exchange name lookup (e.g. 3 -> "NewYorkStockExchange").
 *  @param code The exchange code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_exchange_name(int32_t code);

/** Exchange MIC-like symbol lookup (e.g. 3 -> "NYSE").
 *  @param code The exchange code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_exchange_symbol(int32_t code);

/** Calendar day-type vocabulary lookup (0 -> "open", 1 -> "early_close",
 *  2 -> "full_close", 3 -> "weekend").
 *  @param code The calendar day-type code.
 *  @return A process-lifetime string; "UNKNOWN" for unrecognised codes.
 *          Never NULL; DO NOT FREE. */
const char* thetadatadx_calendar_status_name(int32_t code);

/** Combine an Eastern-Time YYYYMMDD date and milliseconds-of-day into
 *  Unix epoch milliseconds (UTC, DST-aware). Usable with any
 *  (date, *_ms_of_day) pair on the tick structs.
 *  @param date An Eastern-Time YYYYMMDD date.
 *  @param ms_of_day Milliseconds since midnight Eastern.
 *  @return Unix epoch milliseconds, or -1 when date is not a valid YYYYMMDD
 *          (including the 0 absent fill) or ms_of_day is outside
 *          0..86,400,000. */
int64_t thetadatadx_timestamp_ms(int32_t date, int32_t ms_of_day);

/** Convert a signed wire-encoded trade-sequence value to its unsigned
 *  monotonic form.
 *  @param signed_value Wire value; must lie in the 32-bit signed wire range
 *                      (-2,147,483,648 ..= 2,147,483,647).
 *  @param out Receives the unsigned monotonic value on success.
 *  @return 0 on success; -1 with thetadatadx_last_error_code set to
 *          THETADATADX_ERR_INVALID_PARAMETER when signed_value is outside the wire
 *          range or out is null, so an out-of-range value is rejected rather
 *          than silently reinterpreted. */
int32_t thetadatadx_sequence_signed_to_unsigned(int64_t signed_value, uint64_t* out);

/** Convert an unsigned monotonic trade-sequence value back to its signed
 *  wire encoding.
 *  @param unsigned_value Monotonic value; must lie in the unsigned wire
 *                        range (0 ..= 2^32 - 1).
 *  @param out Receives the signed wire value on success.
 *  @return 0 on success; -1 with thetadatadx_last_error_code set to
 *          THETADATADX_ERR_INVALID_PARAMETER when unsigned_value is above the wire
 *          range or out is null. */
int32_t thetadatadx_sequence_unsigned_to_signed(uint64_t unsigned_value, int64_t* out);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Streaming — C-layout event types                                      */
/* ═══════════════════════════════════════════════════════════════════════ */

/* Streaming event structs are schema-driven. The include below pulls in
 * the typedefs generated at build time from the canonical wire schema — so
 * the C++ header can never drift from it again. See `thetadatadx.hpp` for
 * `static_assert(offsetof)` guards that fail the build at compile time if
 * the schema and the C++ consumer ever disagree.
 *
 * Each variant is a typed fixed-layout struct. Consumers dispatch via
 * `event->kind` and read the matching `event-><variant>` payload —
 * for example
 *
 *   if (event->kind == THETADATADX_STREAM_LOGIN_SUCCESS)
 *       printf("perms=%s\n", event->login_success.permissions);
 *   if (event->kind == THETADATADX_STREAM_DISCONNECTED)
 *       printf("reason=%d\n", event->disconnected.reason);
 *
 * Borrowed pointers (`Contract.symbol`, `LoginSuccess.permissions`,
 * `ServerError.message`, `Error.message`, `Ping.payload`,
 * `UnknownFrame.payload`) are valid only for the duration of the
 * user callback — copy out before returning. Do NOT free. */
#include "fpss_event_structs.h.inc"

/** Read the option strike of a streaming ThetaDataDxContract in dollars, folding
 *  the has_strike presence flag into the return value. ThetaDataDxContract.strike
 *  already carries dollars; this surfaces the presence flag a plain field
 *  read would drop. Mirrors the C++ thetadatadx::strike(const ThetaDataDxContract&) accessor.
 *  @param contract The streaming contract to read.
 *  @param out_dollars Receives the strike in dollars when the contract is an
 *                     option; left untouched otherwise.
 *  @return true when the contract is an option and out_dollars was written;
 *          false for a non-option, null contract, or null output pointer. */
bool thetadatadx_contract_strike_dollars(const ThetaDataDxContract* contract, double* out_dollars);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Real-time streaming client                                            */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Connect to the real-time streaming servers.
 *  @param creds Credentials handle; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A streaming handle the caller must release with thetadatadx_streaming_free, or
 *          NULL on failure (check thetadatadx_last_error()). */
ThetaDataDxStreamHandle* thetadatadx_streaming_connect(const ThetaDataDxCredentials* creds, const ThetaDataDxConfig* config);

/** Connect to the real-time streaming servers, reading credentials from a
 *  file (line 1 = email, line 2 = password). One-call equivalent of
 *  thetadatadx_credentials_from_file + thetadatadx_streaming_connect.
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A streaming handle the caller must release with thetadatadx_streaming_free, or
 *          NULL on failure (check thetadatadx_last_error()). */
ThetaDataDxStreamHandle* thetadatadx_streaming_connect_from_file(const char* path, const ThetaDataDxConfig* config);

/** Polymorphic subscribe / unsubscribe — see ThetaDataDxSubscriptionRequest below. */

/** Report whether the streaming connection is currently open. Distinct
 *  from thetadatadx_streaming_is_authenticated: the connection can be open yet
 *  briefly unauthenticated mid-reconnect.
 *  @param h The streaming handle.
 *  @return 1 when streaming, 0 otherwise (including after shutdown). */
int thetadatadx_streaming_is_streaming(const ThetaDataDxStreamHandle* h);

/** Report whether the streaming session is authenticated.
 *  @param h The streaming handle.
 *  @return 1 when authenticated, 0 otherwise. */
int thetadatadx_streaming_is_authenticated(const ThetaDataDxStreamHandle* h);

/** Read the active subscriptions as a typed array.
 *  @param h The streaming handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free. */
ThetaDataDxSubscriptionArray* thetadatadx_streaming_active_subscriptions(const ThetaDataDxStreamHandle* h);

/** Read the active full-stream subscriptions as a typed array. Each
 *  entry's `contract` field carries the security-type discriminant
 *  (`"Stock"` / `"Option"` / `"Index"`); the `kind` field is the
 *  full-stream label (`"full_trades"` / `"full_open_interest"`).
 *  @param h The streaming handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free. */
ThetaDataDxSubscriptionArray* thetadatadx_streaming_active_full_subscriptions(const ThetaDataDxStreamHandle* h);

/** User callback signature for thetadatadx_*_set_callback.
 *  `event` is valid only for the duration of the call -- copy any fields the
 *  caller wants to outlive the callback. `ctx` is the opaque pointer the
 *  caller registered alongside the callback; it is passed back unchanged.
 *
 *  This callback must not unwind across the C ABI. A C++ throw or a C longjmp
 *  that escapes the callback into the calling frame is undefined behavior, the
 *  same as for any C library. The library wraps each invocation to contain a
 *  fault on its own side of the boundary, but that does not contain an
 *  exception thrown out of your callback. Catch and handle every exception
 *  before returning. (The C++ wrapper's set_callback handles this for you: its
 *  shim is noexcept and swallows any exception the std::function raises.) */
typedef void (*ThetaDataDxStreamCallback)(const ThetaDataDxStreamEvent* event, void* ctx);

/** Register a streaming callback and open the streaming connection.
 *
 *  Events flow from the streaming reader through a bounded ring to a
 *  dedicated consumer thread, which invokes the callback inside an
 *  isolation boundary. The reader thread NEVER blocks on user code:
 *  on ring overflow events are dropped and counted (thetadatadx_streaming_dropped_events).
 *
 *  ## ctx lifetime + thread affinity
 *
 *  `ctx` MUST remain valid until ONE of: (a) thetadatadx_streaming_free() returns
 *  (which performs shutdown if needed and applies the drain barrier
 *  internally with a 5 s timeout), or (b) thetadatadx_streaming_shutdown() /
 *  thetadatadx_streaming_reconnect() returns AND thetadatadx_streaming_await_drain() has
 *  returned 1. The consumer thread accesses ctx on every event and on
 *  every thetadatadx_streaming_reconnect(), serially on a single thread. Freeing ctx
 *  without one of these barriers is undefined behavior.
 *
 *  The consumer thread invokes `callback(event, ctx)` serially on
 *  a single thread. The user does NOT need internal locks for callback-
 *  private state.
 *
 *  ## Lifecycle contract (one-shot rule)
 *
 *  Must be called exactly ONCE per handle. After thetadatadx_streaming_shutdown() this
 *  handle is terminal: a second register, a register-after-shutdown, a
 *  reconnect-after-shutdown, or a double-shutdown all return -1 with a
 *  clear thetadatadx_last_error() string ("streaming callback already installed -- ..."
 *  or "streaming handle has already been shut down -- this is terminal").
 *
 *  This is intentionally stricter than thetadatadx_client_set_callback(), where
 *  set-after-stop is supported as a normal user flow.
 *
 *  @param h        The streaming handle.
 *  @param callback The callback invoked once per streaming event.
 *  @param ctx      Opaque user pointer passed to every callback invocation.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_streaming_set_callback(const ThetaDataDxStreamHandle* h, ThetaDataDxStreamCallback callback, void* ctx);

/** Reconnect the streaming session using the previously-registered
 *  callback.
 *  @param h The streaming handle.
 *  @return 0 on success, -1 on error; -1 with "streaming handle has already
 *          been shut down -- this is terminal" if the handle is past
 *          thetadatadx_streaming_shutdown. */
int thetadatadx_streaming_reconnect(const ThetaDataDxStreamHandle* h);

/** Cumulative count of streaming events that could not be published into
 *  the bounded ring because the consumer fell behind and the ring was full.
 *  @param h The streaming handle.
 *  @return The dropped-event count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_streaming_dropped_events(const ThetaDataDxStreamHandle* h);

/** Point-in-time count of streaming events published into the event ring
 *  but not yet drained into the registered callback — the in-flight depth
 *  between the feed and the dispatcher. Rising occupancy that approaches
 *  thetadatadx_streaming_ring_capacity predicts drops before thetadatadx_streaming_dropped_events
 *  moves; sampling never blocks the feed and is safe from any thread.
 *  @param h The streaming handle.
 *  @return The current ring occupancy, or 0 if the handle is null or has
 *          been shut down. */
uint64_t thetadatadx_streaming_ring_occupancy(const ThetaDataDxStreamHandle* h);

/** Configured capacity of the streaming event ring in slots (the
 *  streaming_ring_size setting, a power of two) — the fixed denominator for
 *  thetadatadx_streaming_ring_occupancy.
 *  @param h The streaming handle.
 *  @return The ring capacity in slots, or 0 if the handle is null or has
 *          been shut down. */
uint64_t thetadatadx_streaming_ring_capacity(const ThetaDataDxStreamHandle* h);

/** Milliseconds since the most recent inbound streaming frame of any kind
 *  on this streaming handle.
 *  @param h The streaming handle.
 *  @param out_ms Receives the elapsed milliseconds on success.
 *  @return 0 on success with the value in *out_ms, 1 when no session is live
 *          or no frame has been received yet, -1 on a null pointer. */
int32_t thetadatadx_streaming_millis_since_last_event(const ThetaDataDxStreamHandle* h, uint64_t* out_ms);

/** UNIX-nanosecond receive timestamp of the most recent inbound streaming
 *  frame of any kind on this streaming handle.
 *  @param h The streaming handle.
 *  @return The receive timestamp in Unix nanoseconds, or 0 when the handle
 *          is null, no session is live, or no frame has been received yet. */
int64_t thetadatadx_streaming_last_event_received_at_unix_nanos(const ThetaDataDxStreamHandle* h);

/** Address (host:port) of the streaming server the current session is
 *  connected to, following the session across auto-reconnects.
 *  @param h The streaming handle.
 *  @return A heap-owned C string the caller must release with
 *          thetadatadx_string_free, or NULL when no session is live. */
char* thetadatadx_streaming_last_connected_addr(const ThetaDataDxStreamHandle* h);



/** Cumulative count of user-callback failures contained by the
 *  per-invocation isolation boundary since the current stream started. If
 *  the callback aborts on a given event, the failure is contained, recorded
 *  here, and does not stop event delivery — the next event continues
 *  normally. Safe to call from any thread.
 *  @param h The streaming handle.
 *  @return The contained-failure count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_streaming_panic_count(const ThetaDataDxStreamHandle* h);

/** Shut down the streaming client. Terminal: every subsequent
 *  set_callback / reconnect / shutdown call on this handle returns -1
 *  with a clear thetadatadx_last_error() string. The handle remains valid for
 *  thetadatadx_streaming_free() only. Returns asynchronously: in-flight events
 *  continue draining through the registered callback until the shutdown
 *  signal is observed.
 *  @param h The streaming handle.
 *  @note Pair with thetadatadx_streaming_await_drain() (or use thetadatadx_streaming_free(), which
 *        applies the drain barrier internally) before freeing the callback
 *        ctx. */
void thetadatadx_streaming_shutdown(const ThetaDataDxStreamHandle* h);

/** Wait for the previously-superseded streaming session to quiesce.
 *  @param h The streaming handle.
 *  @param timeout_ms Maximum time to wait, in milliseconds.
 *  @return 1 once the previous thetadatadx_streaming_reconnect / thetadatadx_streaming_shutdown
 *          session has finished firing the registered callback; 0 on timeout
 *          or when no session has been superseded on this handle.
 *  @note Must be called from a thread other than the streaming consumer;
 *        calling it from inside the user callback would block the very work
 *        it waits on and always time out. */
int thetadatadx_streaming_await_drain(const ThetaDataDxStreamHandle* h, uint64_t timeout_ms);

/** Free the streaming handle. Accepts the handle in either lifecycle state:
 *  if shutdown has not yet been called, this performs the shutdown sequence
 *  itself. Returns only after the registered callback has finished firing
 *  (internal 5-second drain barrier). On drain timeout it logs an error and
 *  proceeds with destruction; in that diagnostic case the callback may still
 *  be firing, so user code must keep ctx valid past return. Under normal
 *  operation drain completes in low single-digit milliseconds, so ctx is
 *  safe to free immediately on return.
 *  @param h The streaming handle; no-op when NULL. Call exactly once. */
void thetadatadx_streaming_free(ThetaDataDxStreamHandle* h);

/* ======================================================================= */
/*  Unified client -- market-data + streaming through one handle            */
/* ======================================================================= */

/** Connect to ThetaData (market-data only -- real-time streaming is NOT started).
 *  @param creds Credentials handle; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A unified handle the caller must release with thetadatadx_client_free, or
 *          NULL on connection/auth failure (check thetadatadx_last_error()). */
ThetaDataDxClient* thetadatadx_client_connect(const ThetaDataDxCredentials* creds, const ThetaDataDxConfig* config);

/** Connect a unified client, reading credentials from a file (line 1 = email,
 *  line 2 = password). One-call equivalent of thetadatadx_credentials_from_file +
 *  thetadatadx_client_connect.
 *  @param path Filesystem path to the credentials file; must be non-NULL.
 *  @param config Config handle; must be non-NULL.
 *  @return A unified handle the caller must release with thetadatadx_client_free, or
 *          NULL on argument or connection/auth failure (check
 *          thetadatadx_last_error()). */
ThetaDataDxClient* thetadatadx_client_connect_from_file(const char* path, const ThetaDataDxConfig* config);

/** Register a streaming callback and start streaming on the unified client.
 *
 *  Events flow from the streaming reader through a bounded ring to a
 *  dedicated consumer thread, which invokes the callback inside an
 *  isolation boundary. Reader never blocks on user code; ring-overflow
 *  events are dropped (thetadatadx_client_dropped_events).
 *
 *  ## ctx lifetime + thread affinity
 *
 *  `ctx` MUST remain valid until ONE of: (a) thetadatadx_client_free()
 *  returns (which calls stop_streaming and applies the drain barrier
 *  internally with a 5 s timeout), (b) thetadatadx_client_stop_streaming() /
 *  thetadatadx_client_reconnect() returns AND thetadatadx_client_await_drain() has
 *  returned 1, or (c) a successful replacement thetadatadx_client_set_callback
 *  has returned AND thetadatadx_client_await_drain() has returned 1 for the
 *  prior session. The consumer thread accesses ctx on every event and
 *  reconnect, serially on a single thread. Freeing ctx without one of
 *  these barriers is undefined behavior.
 *
 *  ## Lifecycle contract (REPLACEMENT after stop)
 *
 *  Unlike thetadatadx_streaming_set_callback (one-shot), the unified path supports
 *  stop+register as a normal user flow: after thetadatadx_client_stop_streaming
 *  another thetadatadx_client_set_callback REPLACES the saved (callback, ctx).
 *  thetadatadx_client_reconnect is built on top of this. Calling set_callback
 *  while streaming is already active returns -1 with "streaming already
 *  started".
 *
 *  @param handle   The unified handle.
 *  @param callback The callback invoked once per streaming event.
 *  @param ctx      Opaque user pointer passed to every callback invocation.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_set_callback(const ThetaDataDxClient* handle, ThetaDataDxStreamCallback callback, void* ctx);

/** Subscription request scope discriminator (ThetaDataDxSubscriptionRequest.scope). */
#define THETADATADX_SUB_SCOPE_CONTRACT 0
#define THETADATADX_SUB_SCOPE_FULL     1

/** Subscription kind discriminator (ThetaDataDxSubscriptionRequest.kind). */
#define THETADATADX_SUB_KIND_QUOTE         0
#define THETADATADX_SUB_KIND_TRADE         1
#define THETADATADX_SUB_KIND_OPEN_INTEREST 2
#define THETADATADX_SUB_KIND_MARKET_VALUE  3

/** Polymorphic subscribe / unsubscribe request payload.
 *
 * One payload type expresses every subscription shape across the C ABI.
 *
 * - Per-contract stock: scope=CONTRACT, symbol="AAPL", option fields NULL.
 * - Per-contract option: scope=CONTRACT, symbol="SPY", expiration / strike / right set.
 * - Full-stream: scope=FULL, sec_type="OPTION" (or "STOCK", "INDEX"), per-contract fields NULL.
 */
typedef struct {
    int32_t scope;            /* THETADATADX_SUB_SCOPE_CONTRACT or THETADATADX_SUB_SCOPE_FULL */
    int32_t kind;             /* THETADATADX_SUB_KIND_QUOTE / _TRADE / _OPEN_INTEREST / _MARKET_VALUE */
    const char* symbol;       /* per-contract only */
    const char* expiration;   /* per-contract option only */
    const char* strike;       /* per-contract option only */
    const char* right;        /* per-contract option only */
    const char* sec_type;     /* full-stream only */
} ThetaDataDxSubscriptionRequest;

/** Polymorphic subscribe on the unified client.
 *  @param handle The unified handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_subscribe(const ThetaDataDxClient* handle, const ThetaDataDxSubscriptionRequest* request);

/** Polymorphic unsubscribe on the unified client.
 *  @param handle The unified handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_unsubscribe(const ThetaDataDxClient* handle, const ThetaDataDxSubscriptionRequest* request);

/** Polymorphic subscribe on the standalone streaming client.
 *  @param h The streaming handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_streaming_subscribe(const ThetaDataDxStreamHandle* h, const ThetaDataDxSubscriptionRequest* request);

/** Polymorphic unsubscribe on the standalone streaming client.
 *  @param h The streaming handle.
 *  @param request The subscription request payload.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_streaming_unsubscribe(const ThetaDataDxStreamHandle* h, const ThetaDataDxSubscriptionRequest* request);

/** Reconnect unified streaming, re-subscribing all previous subscriptions.
 *  @param handle The unified handle.
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_client_reconnect(const ThetaDataDxClient* handle);

/** Report whether streaming is active on the unified client.
 *  @param handle The unified handle.
 *  @return 1 when streaming, 0 otherwise. */
int thetadatadx_client_is_streaming(const ThetaDataDxClient* handle);

/** Report whether the live streaming session is authenticated on the
 *  unified client. Distinct from thetadatadx_client_is_streaming: the session
 *  can be live yet briefly unauthenticated mid-reconnect.
 *  @param handle The unified handle.
 *  @return 1 when authenticated, 0 otherwise. */
int thetadatadx_client_is_authenticated(const ThetaDataDxClient* handle);

/** Read the active subscriptions as a typed array.
 *  @param handle The unified handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free. */
ThetaDataDxSubscriptionArray* thetadatadx_client_active_subscriptions(const ThetaDataDxClient* handle);

/** Read the active full-stream subscriptions as a typed array. Each entry's
 *  `contract` field carries the security-type discriminant
 *  ("Stock" / "Option" / "Index") the full-stream subscription is bound
 *  to; the `kind` field is the snake_case full-stream kind label
 *  ("full_trades" / "full_open_interest"), matching the Python /
 *  TypeScript `Subscription.kind` accessor.
 *  @param handle The unified handle.
 *  @return A subscription array the caller MUST free with
 *          thetadatadx_subscription_array_free, or NULL on error. */
ThetaDataDxSubscriptionArray* thetadatadx_client_active_full_subscriptions(const ThetaDataDxClient* handle);

/** Borrow the market-data client from a unified handle.
 *  @param handle The unified handle.
 *  @return A borrowed market-data client pointer owned by the unified handle;
 *          do NOT free it. */
const ThetaDataDxMarketDataClient* thetadatadx_client_market_data(const ThetaDataDxClient* handle);

/** Stop streaming on the unified client. Market-data remains available.
 *  Returns asynchronously: in-flight events continue draining through the
 *  registered callback until the shutdown signal is observed.
 *  @param handle The unified handle.
 *  @note Pair with thetadatadx_client_await_drain() (or use thetadatadx_client_free(),
 *        which applies the drain barrier internally) before freeing the
 *        callback ctx. */
void thetadatadx_client_stop_streaming(const ThetaDataDxClient* handle);

/** Wait for the previously-superseded streaming session to quiesce.
 *  @param handle The unified handle.
 *  @param timeout_ms Maximum time to wait, in milliseconds.
 *  @return 1 once the previous session has finished firing the registered
 *          callback, or when no stream has ever been started or stopped on
 *          this handle (an idle handle is already quiesced); 0 only on
 *          timeout. Note the standalone thetadatadx_streaming_await_drain
 *          returns 0 for the idle case instead.
 *  @note Must be called from a thread other than the streaming consumer. */
int thetadatadx_client_await_drain(const ThetaDataDxClient* handle, uint64_t timeout_ms);

/** Cumulative count of streaming events that could not be published into
 *  the bounded ring because the consumer fell behind and the ring was full.
 *  @param handle The unified handle.
 *  @return The dropped-event count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_dropped_events(const ThetaDataDxClient* handle);

/** Point-in-time count of streaming events published into the event ring
 *  but not yet drained into the registered callback — the in-flight depth
 *  between the feed and the dispatcher. Rising occupancy that approaches
 *  thetadatadx_client_ring_capacity predicts drops before thetadatadx_client_dropped_events
 *  moves; sampling never blocks the feed and is safe from any thread.
 *  @param handle The unified handle.
 *  @return The current ring occupancy, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_ring_occupancy(const ThetaDataDxClient* handle);

/** Configured capacity of the streaming event ring in slots (the
 *  streaming_ring_size setting, a power of two) — the fixed denominator for
 *  thetadatadx_client_ring_occupancy.
 *  @param handle The unified handle.
 *  @return The ring capacity in slots, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_ring_capacity(const ThetaDataDxClient* handle);

/** Milliseconds since the most recent inbound streaming frame of any kind
 *  on this unified handle.
 *  @param handle The unified handle.
 *  @param out_ms Receives the elapsed milliseconds on success.
 *  @return 0 on success with the value in *out_ms, 1 when streaming has not
 *          started or no frame has been received yet, -1 on a null pointer. */
int32_t thetadatadx_client_millis_since_last_event(const ThetaDataDxClient* handle, uint64_t* out_ms);

/** UNIX-nanosecond receive timestamp of the most recent inbound streaming
 *  frame of any kind on this unified handle.
 *  @param handle The unified handle.
 *  @return The receive timestamp in Unix nanoseconds, or 0 when the handle
 *          is null, streaming has not started, or no frame has been received
 *          yet. */
int64_t thetadatadx_client_last_event_received_at_unix_nanos(const ThetaDataDxClient* handle);

/** Address (host:port) of the streaming server the current session is
 *  connected to, following the session across auto-reconnects.
 *  @param handle The unified handle.
 *  @return A heap-owned C string the caller must release with
 *          thetadatadx_string_free, or NULL when streaming has not started. */
char* thetadatadx_client_last_connected_addr(const ThetaDataDxClient* handle);



/** Cumulative count of user-callback failures contained by the
 *  per-invocation isolation boundary since the current stream started. If
 *  the callback aborts on a given event, the failure is contained, recorded
 *  here, and does not stop event delivery — the next event continues
 *  normally. Safe to call from any thread.
 *  @param handle The unified handle.
 *  @return The contained-failure count, or 0 if the handle is null or no
 *          callback has been installed yet. */
uint64_t thetadatadx_client_panic_count(const ThetaDataDxClient* handle);

/** Free a unified client handle. Calls thetadatadx_client_stop_streaming
 *  internally, then waits up to 5 seconds for the registered callback to
 *  finish firing before destroying the handle. On drain timeout it logs an
 *  error and proceeds with destruction; in that diagnostic case the callback
 *  may still be firing, so user code must keep ctx valid past return. Under
 *  normal operation drain completes in low single-digit milliseconds, so ctx
 *  is safe to free immediately on return.
 *  @param handle The unified handle; no-op when NULL. Call exactly once. */
void thetadatadx_client_free(ThetaDataDxClient* handle);


/* ── FLATFILES surface ────────────────────────────────────────────────
 *
 * Whole-universe daily snapshots over the legacy market-data-channel
 * port. The schema is determined at runtime by (sec_type, req_type),
 * so the typed decoder returns an opaque row-list handle that you
 * serialise to Arrow IPC bytes when you want columnar output.
 */

/** Opaque handle wrapping a decoded set of flat-file rows. Created by
 *  thetadatadx_flatfile_request_decoded; freed by thetadatadx_flatfile_rowlist_free. */
typedef struct ThetaDataDxFlatFileRowList ThetaDataDxFlatFileRowList;

/** Heap-owned byte buffer (Arrow IPC stream) returned by
 *  thetadatadx_flatfile_rows_to_arrow_ipc. Caller MUST free with
 *  thetadatadx_flatfile_bytes_free. */
typedef struct ThetaDataDxFlatFileBytes {
    const uint8_t* data;
    size_t len;
} ThetaDataDxFlatFileBytes;

/** Pull a decoded flat-file blob for (sec_type, req_type, date) and
 *  return an opaque row-list handle.
 *  @param handle The unified handle.
 *  @param sec_type "OPTION", "STOCK", or "INDEX".
 *  @param req_type "EOD", "QUOTE", "OPEN_INTEREST", "OHLC", "TRADE", or
 *                  "TRADE_QUOTE".
 *  @param date The snapshot date as "YYYYMMDD".
 *  @return A row-list handle the caller MUST free with
 *          thetadatadx_flatfile_rowlist_free, or NULL on error (check
 *          thetadatadx_last_error()). */
ThetaDataDxFlatFileRowList* thetadatadx_flatfile_request_decoded(
    const ThetaDataDxClient* handle,
    const char* sec_type,
    const char* req_type,
    const char* date);

/** Number of rows in a row-list handle.
 *  @param rowlist The row-list handle.
 *  @return The row count, or 0 if rowlist is NULL. */
size_t thetadatadx_flatfile_rows_count(const ThetaDataDxFlatFileRowList* rowlist);

/** Serialise the row list as Arrow IPC stream bytes. The schema is inferred
 *  from the first row.
 *  @param rowlist The row-list handle.
 *  @return An Arrow IPC byte buffer the caller MUST free with
 *          thetadatadx_flatfile_bytes_free, or (data=NULL, len=0) on error (check
 *          thetadatadx_last_error()). */
ThetaDataDxFlatFileBytes thetadatadx_flatfile_rows_to_arrow_ipc(
    const ThetaDataDxFlatFileRowList* rowlist);

/** Free a byte buffer returned by thetadatadx_flatfile_rows_to_arrow_ipc.
 *  @param bytes Buffer from thetadatadx_flatfile_rows_to_arrow_ipc; a (data=NULL,
 *               len=0) buffer is a no-op. Call exactly once. */
void thetadatadx_flatfile_bytes_free(ThetaDataDxFlatFileBytes bytes);

/** Free a row-list handle returned by thetadatadx_flatfile_request_decoded.
 *  @param rowlist Handle from thetadatadx_flatfile_request_decoded; no-op when NULL.
 *                 Call exactly once. */
void thetadatadx_flatfile_rowlist_free(ThetaDataDxFlatFileRowList* rowlist);

/** Pull a flat-file blob and write the requested vendor format directly to
 *  a file. The format extension is appended to path automatically if missing.
 *  @param handle The unified handle.
 *  @param sec_type "OPTION", "STOCK", or "INDEX".
 *  @param req_type "EOD", "QUOTE", "OPEN_INTEREST", "OHLC", "TRADE", or
 *                  "TRADE_QUOTE".
 *  @param date The snapshot date as "YYYYMMDD".
 *  @param path Output file path; the format extension is appended if missing.
 *  @param format Output format: "csv", "json", "jsonl"/"ndjson", or "html".
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_flatfile_request_to_path(
    const ThetaDataDxClient* handle,
    const char* sec_type,
    const char* req_type,
    const char* date,
    const char* path,
    const char* format);

/** Pull a decoded flat-file blob from a standalone market-data client.
 *  Flat files are account-authenticated market data, so the market-data
 *  handle exposes the identical surface as the unified client.
 *  @param handle The market-data handle.
 *  @param sec_type "OPTION", "STOCK", or "INDEX".
 *  @param req_type "EOD", "QUOTE", "OPEN_INTEREST", "OHLC", "TRADE", or
 *                  "TRADE_QUOTE".
 *  @param date The snapshot date as "YYYYMMDD".
 *  @return A row-list handle the caller MUST free with
 *          thetadatadx_flatfile_rowlist_free, or NULL on error (check
 *          thetadatadx_last_error()). */
ThetaDataDxFlatFileRowList* thetadatadx_market_data_flatfile_request_decoded(
    const ThetaDataDxMarketDataClient* handle,
    const char* sec_type,
    const char* req_type,
    const char* date);

/** Pull a flat-file blob from a standalone market-data client and write the
 *  requested vendor format directly to a file. The format extension is
 *  appended to path automatically if missing.
 *  @param handle The market-data handle.
 *  @param sec_type "OPTION", "STOCK", or "INDEX".
 *  @param req_type "EOD", "QUOTE", "OPEN_INTEREST", "OHLC", "TRADE", or
 *                  "TRADE_QUOTE".
 *  @param date The snapshot date as "YYYYMMDD".
 *  @param path Output file path; the format extension is appended if missing.
 *  @param format Output format: "csv", "json", "jsonl"/"ndjson", or "html".
 *  @return 0 on success, -1 on error (check thetadatadx_last_error()). */
int thetadatadx_market_data_flatfile_request_to_path(
    const ThetaDataDxMarketDataClient* handle,
    const char* sec_type,
    const char* req_type,
    const char* date,
    const char* path,
    const char* format);

#ifdef __cplusplus
}
#endif

#endif /* THETADATADX_H */
