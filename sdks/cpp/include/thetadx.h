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
    /* volume/count are int64 to match the core crate (issue #372) and
     * prevent overflow on high-volume symbols (2.1B+ cumulative volume). */
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

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
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
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[48];
} TdxGreeksTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double rate;
    int32_t date;
    uint8_t _tail_padding[40];
} TdxInterestRateTick TDX_ALIGN64_END;

TDX_ALIGN64_BEGIN typedef struct {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double implied_volatility;
    double iv_error;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[16];
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
    /* volume/count are int64 to match the core crate (issue #372) and
     * prevent overflow on high-volume symbols (2.1B+ cumulative volume). */
    int64_t volume;
    int64_t count;
    int32_t date;
    int32_t expiration;
    double strike;
    int32_t right;
    uint8_t _tail_padding[52];
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
typedef struct { const TdxGreeksTick* data; size_t len; } TdxGreeksTickArray;
typedef struct { const TdxIvTick* data; size_t len; } TdxIvTickArray;
typedef struct { const TdxPriceTick* data; size_t len; } TdxPriceTickArray;
typedef struct { const TdxOpenInterestTick* data; size_t len; } TdxOpenInterestTickArray;
typedef struct { const TdxMarketValueTick* data; size_t len; } TdxMarketValueTickArray;
typedef struct { const TdxCalendarDay* data; size_t len; } TdxCalendarDayArray;
typedef struct { const TdxInterestRateTick* data; size_t len; } TdxInterestRateTickArray;
typedef struct { const TdxTradeQuoteTick* data; size_t len; } TdxTradeQuoteTickArray;

/* ── OptionContract (has heap-allocated root string) ── */

typedef struct {
    const char* root;       /* heap-allocated, freed with tdx_option_contract_array_free */
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

typedef struct {
    int32_t id;
    const char* contract;
} TdxContractMapEntry;

typedef struct {
    const TdxContractMapEntry* data;
    size_t len;
} TdxContractMapArray;

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Free functions for typed arrays                                       */
/* ═══════════════════════════════════════════════════════════════════════ */

void tdx_eod_tick_array_free(TdxEodTickArray arr);
void tdx_ohlc_tick_array_free(TdxOhlcTickArray arr);
void tdx_trade_tick_array_free(TdxTradeTickArray arr);
void tdx_quote_tick_array_free(TdxQuoteTickArray arr);
void tdx_greeks_tick_array_free(TdxGreeksTickArray arr);
void tdx_iv_tick_array_free(TdxIvTickArray arr);
void tdx_price_tick_array_free(TdxPriceTickArray arr);
void tdx_open_interest_tick_array_free(TdxOpenInterestTickArray arr);
void tdx_market_value_tick_array_free(TdxMarketValueTickArray arr);
void tdx_calendar_day_array_free(TdxCalendarDayArray arr);
void tdx_interest_rate_tick_array_free(TdxInterestRateTickArray arr);
void tdx_trade_quote_tick_array_free(TdxTradeQuoteTickArray arr);
void tdx_option_contract_array_free(TdxOptionContractArray arr);
void tdx_string_array_free(TdxStringArray arr);
void tdx_greeks_result_free(TdxGreeksResult* result);
void tdx_subscription_array_free(TdxSubscriptionArray* arr);
void tdx_contract_map_array_free(TdxContractMapArray* arr);

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

/** Create a dev FPSS config (port 20200, infinite historical replay). */
TdxConfig* tdx_config_dev(void);

/** Create a stage FPSS config (port 20100, testing, unstable). */
TdxConfig* tdx_config_stage(void);

/** Free a config handle. */
void tdx_config_free(TdxConfig* config);

/**
 * Set FPSS reconnect policy on a config handle.
 *   policy=0: Auto (default) -- auto-reconnect matching Java terminal behavior.
 *   policy=1: Manual -- no auto-reconnect.
 */
void tdx_config_set_reconnect_policy(TdxConfig* config, int policy);

/**
 * Set FPSS flush mode on a config handle.
 *   mode=0: Batched (default) -- flush only on PING every 100ms.
 *   mode=1: Immediate -- flush after every frame write.
 */
void tdx_config_set_flush_mode(TdxConfig* config, int mode);

/**
 * Set FPSS OHLCVC derivation on a config handle.
 *   enabled=1 (default): derive OHLCVC bars locally from trade events.
 *   enabled=0: only emit server-sent OHLCVC frames (lower overhead).
 */
void tdx_config_set_derive_ohlcvc(TdxConfig* config, int enabled);

/* ── Client ── */

/** Connect to ThetaData servers. Returns NULL on connection/auth failure. */
TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Free a client handle. */
void tdx_client_free(TdxClient* client);

/* ── String free ── */

/** Free a string returned by any tdx_* function. */
void tdx_string_free(char* s);

/* Generated option-aware endpoint declarations. */
#include "endpoint_with_options.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Greeks (standalone)                                                   */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Compute all 22 Greeks + IV. `right` accepts "C"/"P" or "call"/"put" (case-insensitive).
 *  Returns heap-allocated TdxGreeksResult (or NULL on error). Caller must free with tdx_greeks_result_free. */
TdxGreeksResult* tdx_all_greeks(double spot, double strike, double rate, double div_yield,
                                double tte, double option_price, const char* right);

/** Compute implied volatility. `right` accepts "C"/"P" or "call"/"put" (case-insensitive).
 *  Returns 0 on success, -1 on failure. */
int tdx_implied_volatility(double spot, double strike, double rate, double div_yield,
                           double tte, double option_price, const char* right,
                           double* out_iv, double* out_error);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  FPSS — #[repr(C)] streaming event types                               */
/* ═══════════════════════════════════════════════════════════════════════ */

/* FPSS event structs are schema-driven. The include below pulls in the
 * same typedefs the Go SDK uses, generated from
 * `crates/thetadatadx/fpss_event_schema.toml` — so the C++ header can
 * never drift from the Rust `#[repr(C)]` layout again. See
 * `thetadx.hpp` for `static_assert(offsetof)` guards that fail the
 * build at compile time if the schema and the C++ consumer ever
 * disagree.
 *
 * `TdxFpssControl::kind` encodes the sub-type:
 *   0=login_success, 1=contract_assigned, 2=req_response,
 *   3=market_open, 4=market_close, 5=server_error,
 *   6=disconnected, 8=reconnecting, 9=reconnected,
 *   10=error, 11=unknown_frame, 12=unknown_event
 * (value 7 is reserved). `id` carries the contract_id / req_id /
 * reconnect attempt where applicable (0 otherwise). `detail` is a
 * NUL-terminated string; may be NULL. Do NOT free. */
#include "fpss_event_structs.h.inc"

/* ═══════════════════════════════════════════════════════════════════════ */
/*  FPSS — Real-time streaming client                                     */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Connect to FPSS streaming servers. Returns NULL on failure. */
TdxFpssHandle* tdx_fpss_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Subscribe to quote data. Returns request ID or -1 on error. */
int tdx_fpss_subscribe_quotes(const TdxFpssHandle* h, const char* symbol);

/** Subscribe to trade data. Returns request ID or -1 on error. */
int tdx_fpss_subscribe_trades(const TdxFpssHandle* h, const char* symbol);

/** Subscribe to open interest data. Returns request ID or -1 on error. */
int tdx_fpss_subscribe_open_interest(const TdxFpssHandle* h, const char* symbol);

/** Subscribe to all trades for a security type. sec_type: "STOCK", "OPTION", "INDEX". Returns request ID or -1. */
int tdx_fpss_subscribe_full_trades(const TdxFpssHandle* h, const char* sec_type);

/** Subscribe to all open interest for a security type. sec_type: "STOCK", "OPTION", "INDEX". Returns request ID or -1. */
int tdx_fpss_subscribe_full_open_interest(const TdxFpssHandle* h, const char* sec_type);

/** Unsubscribe from all trades for a security type. sec_type: "STOCK", "OPTION", "INDEX". Returns request ID or -1. */
int tdx_fpss_unsubscribe_full_trades(const TdxFpssHandle* h, const char* sec_type);

/** Unsubscribe from all open interest for a security type. sec_type: "STOCK", "OPTION", "INDEX". Returns request ID or -1. */
int tdx_fpss_unsubscribe_full_open_interest(const TdxFpssHandle* h, const char* sec_type);

/** Unsubscribe from quote data. Returns request ID or -1 on error. */
int tdx_fpss_unsubscribe_quotes(const TdxFpssHandle* h, const char* symbol);

/** Unsubscribe from trade data. Returns request ID or -1 on error. */
int tdx_fpss_unsubscribe_trades(const TdxFpssHandle* h, const char* symbol);

/** Unsubscribe from open interest data. Returns request ID or -1 on error. */
int tdx_fpss_unsubscribe_open_interest(const TdxFpssHandle* h, const char* symbol);

/** Subscribe to quote data for an option contract. Returns 0 or -1. */
int tdx_fpss_subscribe_option_quotes(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Subscribe to trade data for an option contract. Returns 0 or -1. */
int tdx_fpss_subscribe_option_trades(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Subscribe to open interest for an option contract. Returns 0 or -1. */
int tdx_fpss_subscribe_option_open_interest(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Unsubscribe from quote data for an option contract. Returns 0 or -1. */
int tdx_fpss_unsubscribe_option_quotes(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Unsubscribe from trade data for an option contract. Returns 0 or -1. */
int tdx_fpss_unsubscribe_option_trades(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Unsubscribe from open interest for an option contract. Returns 0 or -1. */
int tdx_fpss_unsubscribe_option_open_interest(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Check if authenticated. Returns 1 if true, 0 if false. */
int tdx_fpss_is_authenticated(const TdxFpssHandle* h);

/** Look up a contract by server-assigned ID. Returns string or NULL.
 *  NULL with empty tdx_last_error() means "not found". NULL with non-empty
 *  tdx_last_error() means a real error occurred. Caller must free with tdx_string_free. */
char* tdx_fpss_contract_lookup(const TdxFpssHandle* h, int id);

/** Get the full contract map as typed entries. Caller must free with tdx_contract_map_array_free. */
TdxContractMapArray* tdx_fpss_contract_map(const TdxFpssHandle* h);

/** Get active subscriptions as typed array. Caller must free with tdx_subscription_array_free. */
TdxSubscriptionArray* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);

/** Poll for the next event as a typed struct. Returns TdxFpssEvent* or NULL on timeout.
 *  Caller MUST free with tdx_fpss_event_free. */
TdxFpssEvent* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);

/** Free a TdxFpssEvent returned by tdx_fpss_next_event. */
void tdx_fpss_event_free(TdxFpssEvent* event);

/** Reconnect FPSS, re-subscribing all previous subscriptions. Returns 0 or -1. */
int tdx_fpss_reconnect(const TdxFpssHandle* h);

/** Cumulative count of FPSS events dropped because the internal receiver
 *  was gone when the callback tried to deliver. Survives reconnect.
 *  Returns 0 if the handle is null. */
uint64_t tdx_fpss_dropped_events(const TdxFpssHandle* h);

/** Shut down the FPSS client. */
void tdx_fpss_shutdown(const TdxFpssHandle* h);

/** Free the FPSS handle. Must be called after tdx_fpss_shutdown. */
void tdx_fpss_free(TdxFpssHandle* h);

/* ======================================================================= */
/*  Unified client -- historical + streaming through one handle            */
/* ======================================================================= */

/** Connect to ThetaData (historical only -- FPSS streaming is NOT started).
 *  Returns NULL on connection/auth failure (check tdx_last_error()). */
TdxUnified* tdx_unified_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Start FPSS streaming on the unified client. Returns 0 on success, -1 on error. */
int tdx_unified_start_streaming(const TdxUnified* handle);

/** Subscribe to quote data for a stock symbol. Returns 0 on success, -1 on error. */
int tdx_unified_subscribe_quotes(const TdxUnified* handle, const char* symbol);

/** Subscribe to trade data for a stock symbol. Returns 0 on success, -1 on error. */
int tdx_unified_subscribe_trades(const TdxUnified* handle, const char* symbol);

/** Unsubscribe from quote data. Returns 0 on success, -1 on error. */
int tdx_unified_unsubscribe_quotes(const TdxUnified* handle, const char* symbol);

/** Unsubscribe from trade data. Returns 0 on success, -1 on error. */
int tdx_unified_unsubscribe_trades(const TdxUnified* handle, const char* symbol);

/** Subscribe to open interest data. Returns 0 on success, -1 on error. */
int tdx_unified_subscribe_open_interest(const TdxUnified* handle, const char* symbol);

/** Unsubscribe from open interest data. Returns 0 on success, -1 on error. */
int tdx_unified_unsubscribe_open_interest(const TdxUnified* handle, const char* symbol);

/** Subscribe to all trades for a security type ("STOCK", "OPTION", "INDEX"). Returns 0 or -1. */
int tdx_unified_subscribe_full_trades(const TdxUnified* handle, const char* sec_type);

/** Subscribe to all open interest for a security type. Returns 0 or -1. */
int tdx_unified_subscribe_full_open_interest(const TdxUnified* handle, const char* sec_type);

/** Unsubscribe from all trades for a security type. Returns 0 or -1. */
int tdx_unified_unsubscribe_full_trades(const TdxUnified* handle, const char* sec_type);

/** Unsubscribe from all open interest for a security type. Returns 0 or -1. */
int tdx_unified_unsubscribe_full_open_interest(const TdxUnified* handle, const char* sec_type);

/** Subscribe to quote data for an option contract. Returns 0 or -1. */
int tdx_unified_subscribe_option_quotes(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Subscribe to trade data for an option contract. Returns 0 or -1. */
int tdx_unified_subscribe_option_trades(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Subscribe to open interest for an option contract. Returns 0 or -1. */
int tdx_unified_subscribe_option_open_interest(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Unsubscribe from quote data for an option contract. Returns 0 or -1. */
int tdx_unified_unsubscribe_option_quotes(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Unsubscribe from trade data for an option contract. Returns 0 or -1. */
int tdx_unified_unsubscribe_option_trades(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Unsubscribe from open interest for an option contract. Returns 0 or -1. */
int tdx_unified_unsubscribe_option_open_interest(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);

/** Get the full contract map as typed entries. Caller must free with tdx_contract_map_array_free. */
TdxContractMapArray* tdx_unified_contract_map(const TdxUnified* handle);

/** Reconnect unified streaming, re-subscribing all previous subscriptions. Returns 0 or -1. */
int tdx_unified_reconnect(const TdxUnified* handle);

/** Check if streaming is active. Returns 1 if streaming, 0 otherwise. */
int tdx_unified_is_streaming(const TdxUnified* handle);

/** Look up a contract by ID. Returns string or NULL.
 *  NULL with empty tdx_last_error() means "not found". NULL with non-empty
 *  tdx_last_error() means a real error occurred. Caller must free with tdx_string_free. */
char* tdx_unified_contract_lookup(const TdxUnified* handle, int id);

/** Get active subscriptions as typed array. Caller must free with tdx_subscription_array_free. */
TdxSubscriptionArray* tdx_unified_active_subscriptions(const TdxUnified* handle);

/** Poll for next streaming event. Returns TdxFpssEvent* or NULL on timeout.
 *  Caller MUST free with tdx_fpss_event_free. */
TdxFpssEvent* tdx_unified_next_event(const TdxUnified* handle, uint64_t timeout_ms);

/** Borrow the historical client from a unified handle. Do NOT free the returned pointer. */
const TdxClient* tdx_unified_historical(const TdxUnified* handle);

/** Stop streaming on the unified client. Historical remains available. */
void tdx_unified_stop_streaming(const TdxUnified* handle);

/** Cumulative count of FPSS events dropped because the unified handle's
 *  internal receiver was gone when the callback tried to deliver.
 *  Survives reconnect. Returns 0 if the handle is null. */
uint64_t tdx_unified_dropped_events(const TdxUnified* handle);

/** Free a unified client handle. */
void tdx_unified_free(TdxUnified* handle);

#ifdef __cplusplus
}
#endif

#endif /* THETADX_H */
