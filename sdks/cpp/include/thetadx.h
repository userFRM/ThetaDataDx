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

#ifdef __cplusplus
extern "C" {
#endif

/* ── Opaque handle types ── */
typedef struct TdxCredentials TdxCredentials;
typedef struct TdxClient TdxClient;
typedef struct TdxConfig TdxConfig;
typedef struct TdxFpssHandle TdxFpssHandle;

/* ═══════════════════════════════════════════════════════════════════════ */
/*  #[repr(C)] tick types — layout-compatible with Rust tdbe structs      */
/* ═══════════════════════════════════════════════════════════════════════ */

/* All tick structs are 64-byte aligned to match Rust's #[repr(C, align(64))]. */

typedef struct __attribute__((aligned(64))) {
    int32_t date;
    int32_t is_open;
    int32_t open_time;
    int32_t close_time;
    int32_t status;
} TdxCalendarDay;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t ms_of_day2;
    int32_t open;
    int32_t high;
    int32_t low;
    int32_t close;
    int32_t volume;
    int32_t count;
    int32_t bid_size;
    int32_t bid_exchange;
    int32_t bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    int32_t ask;
    int32_t ask_condition;
    int32_t price_type;
    int32_t date;
} TdxEodTick;

typedef struct __attribute__((aligned(64))) {
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
} TdxGreeksTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double rate;
    int32_t date;
} TdxInterestRateTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    /* 4 bytes padding before f64 */
    double implied_volatility;
    double iv_error;
    int32_t date;
} TdxIvTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    /* 4 bytes padding before i64 */
    int64_t market_cap;
    int64_t shares_outstanding;
    int64_t enterprise_value;
    int64_t book_value;
    int64_t free_float;
    int32_t date;
} TdxMarketValueTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t open;
    int32_t high;
    int32_t low;
    int32_t close;
    int32_t volume;
    int32_t count;
    int32_t price_type;
    int32_t date;
} TdxOhlcTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t open_interest;
    int32_t date;
} TdxOpenInterestTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t price;
    int32_t price_type;
    int32_t date;
} TdxPriceTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    int32_t bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    int32_t ask;
    int32_t ask_condition;
    int32_t price_type;
    int32_t date;
} TdxQuoteTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t size;
    int32_t condition;
    int32_t price;
    int32_t price_type;
    int32_t date;
} TdxSnapshotTradeTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    int32_t price;
    int32_t condition_flags;
    int32_t price_flags;
    int32_t volume_type;
    int32_t records_back;
    int32_t quote_ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    int32_t bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    int32_t ask;
    int32_t ask_condition;
    int32_t quote_price_type;
    int32_t price_type;
    int32_t date;
} TdxTradeQuoteTick;

typedef struct __attribute__((aligned(64))) {
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    int32_t price;
    int32_t condition_flags;
    int32_t price_flags;
    int32_t volume_type;
    int32_t records_back;
    int32_t price_type;
    int32_t date;
} TdxTradeTick;

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
typedef struct { const TdxSnapshotTradeTick* data; size_t len; } TdxSnapshotTradeTickArray;
typedef struct { const TdxTradeQuoteTick* data; size_t len; } TdxTradeQuoteTickArray;

/* ── OptionContract (has heap-allocated root string) ── */

typedef struct {
    const char* root;       /* heap-allocated, freed with tdx_option_contract_array_free */
    int32_t expiration;
    int32_t strike;
    int32_t right;
    int32_t strike_price_type;
} TdxOptionContract;

typedef struct { const TdxOptionContract* data; size_t len; } TdxOptionContractArray;

/* ── String array (for list endpoints) ── */

typedef struct {
    const char* const* data;  /* array of NUL-terminated C strings */
    size_t len;
} TdxStringArray;

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
void tdx_snapshot_trade_tick_array_free(TdxSnapshotTradeTickArray arr);
void tdx_trade_quote_tick_array_free(TdxTradeQuoteTickArray arr);
void tdx_option_contract_array_free(TdxOptionContractArray arr);
void tdx_string_array_free(TdxStringArray arr);

/* ── Error ── */

/** Retrieve the last error message (or NULL if no error).
 *  The returned pointer is valid until the next FFI call on the same thread.
 *  Do NOT free this pointer. */
const char* tdx_last_error(void);

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

/* ── Client ── */

/** Connect to ThetaData servers. Returns NULL on connection/auth failure. */
TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Free a client handle. */
void tdx_client_free(TdxClient* client);

/* ── String free ── */

/** Free a string returned by any tdx_* function. */
void tdx_string_free(char* s);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — List endpoints (2)                                            */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 1. List all stock symbols. Returns TdxStringArray. */
TdxStringArray tdx_stock_list_symbols(const TdxClient* client);

/** 2. List available dates for a stock by request type. Returns TdxStringArray. */
TdxStringArray tdx_stock_list_dates(const TdxClient* client, const char* request_type, const char* symbol);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — Snapshot endpoints (4)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 3. Get latest OHLC snapshot. symbols_json is JSON array. */
TdxOhlcTickArray tdx_stock_snapshot_ohlc(const TdxClient* client, const char* symbols_json);

/** 4. Get latest trade snapshot. symbols_json is JSON array. */
TdxTradeTickArray tdx_stock_snapshot_trade(const TdxClient* client, const char* symbols_json);

/** 5. Get latest NBBO quote snapshot. symbols_json is JSON array. */
TdxQuoteTickArray tdx_stock_snapshot_quote(const TdxClient* client, const char* symbols_json);

/** 6. Get latest market value snapshot. symbols_json is JSON array. */
TdxMarketValueTickArray tdx_stock_snapshot_market_value(const TdxClient* client, const char* symbols_json);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — History endpoints (5 + bonus)                                 */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 7. Fetch stock EOD history. */
TdxEodTickArray tdx_stock_history_eod(const TdxClient* client, const char* symbol,
                                      const char* start_date, const char* end_date);

/** 8. Fetch stock intraday OHLC. */
TdxOhlcTickArray tdx_stock_history_ohlc(const TdxClient* client, const char* symbol,
                                        const char* date, const char* interval);

/** 8b. Fetch stock OHLC across date range. */
TdxOhlcTickArray tdx_stock_history_ohlc_range(const TdxClient* client, const char* symbol,
                                              const char* start_date, const char* end_date,
                                              const char* interval);

/** 9. Fetch all trades on a date. */
TdxTradeTickArray tdx_stock_history_trade(const TdxClient* client, const char* symbol, const char* date);

/** 10. Fetch NBBO quotes. */
TdxQuoteTickArray tdx_stock_history_quote(const TdxClient* client, const char* symbol,
                                          const char* date, const char* interval);

/** 11. Fetch combined trade + quote ticks. */
TdxTradeQuoteTickArray tdx_stock_history_trade_quote(const TdxClient* client, const char* symbol, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — At-Time endpoints (2)                                         */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 12. Fetch trade at a specific time across a date range. */
TdxTradeTickArray tdx_stock_at_time_trade(const TdxClient* client, const char* symbol,
                                          const char* start_date, const char* end_date,
                                          const char* time_of_day);

/** 13. Fetch quote at a specific time across a date range. */
TdxQuoteTickArray tdx_stock_at_time_quote(const TdxClient* client, const char* symbol,
                                          const char* start_date, const char* end_date,
                                          const char* time_of_day);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — List endpoints (5)                                           */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 14. List all option underlyings. */
TdxStringArray tdx_option_list_symbols(const TdxClient* client);

/** 15. List available dates for an option contract. */
TdxStringArray tdx_option_list_dates(const TdxClient* client, const char* request_type,
                                     const char* symbol, const char* expiration,
                                     const char* strike, const char* right);

/** 16. List expiration dates. */
TdxStringArray tdx_option_list_expirations(const TdxClient* client, const char* symbol);

/** 17. List strike prices. */
TdxStringArray tdx_option_list_strikes(const TdxClient* client, const char* symbol,
                                       const char* expiration);

/** 18. List all option contracts on a date. */
TdxOptionContractArray tdx_option_list_contracts(const TdxClient* client, const char* request_type,
                                                 const char* symbol, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — Snapshot endpoints (10)                                      */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 19. Get latest OHLC snapshot for options. */
TdxOhlcTickArray tdx_option_snapshot_ohlc(const TdxClient* client, const char* symbol,
                                          const char* expiration, const char* strike, const char* right);

/** 20. Get latest trade snapshot for options. */
TdxTradeTickArray tdx_option_snapshot_trade(const TdxClient* client, const char* symbol,
                                            const char* expiration, const char* strike, const char* right);

/** 21. Get latest NBBO quote snapshot for options. */
TdxQuoteTickArray tdx_option_snapshot_quote(const TdxClient* client, const char* symbol,
                                            const char* expiration, const char* strike, const char* right);

/** 22. Get latest open interest snapshot. */
TdxOpenInterestTickArray tdx_option_snapshot_open_interest(const TdxClient* client, const char* symbol,
                                                           const char* expiration, const char* strike, const char* right);

/** 23. Get latest market value snapshot for options. */
TdxMarketValueTickArray tdx_option_snapshot_market_value(const TdxClient* client, const char* symbol,
                                                         const char* expiration, const char* strike, const char* right);

/** 24. Get IV snapshot for options. */
TdxIvTickArray tdx_option_snapshot_greeks_implied_volatility(const TdxClient* client, const char* symbol,
                                                             const char* expiration, const char* strike, const char* right);

/** 25. Get all Greeks snapshot. */
TdxGreeksTickArray tdx_option_snapshot_greeks_all(const TdxClient* client, const char* symbol,
                                                   const char* expiration, const char* strike, const char* right);

/** 26. Get first-order Greeks snapshot. */
TdxGreeksTickArray tdx_option_snapshot_greeks_first_order(const TdxClient* client, const char* symbol,
                                                           const char* expiration, const char* strike, const char* right);

/** 27. Get second-order Greeks snapshot. */
TdxGreeksTickArray tdx_option_snapshot_greeks_second_order(const TdxClient* client, const char* symbol,
                                                            const char* expiration, const char* strike, const char* right);

/** 28. Get third-order Greeks snapshot. */
TdxGreeksTickArray tdx_option_snapshot_greeks_third_order(const TdxClient* client, const char* symbol,
                                                           const char* expiration, const char* strike, const char* right);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — History endpoints (6)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 29. Fetch EOD option data. */
TdxEodTickArray tdx_option_history_eod(const TdxClient* client, const char* symbol, const char* expiration,
                                       const char* strike, const char* right,
                                       const char* start_date, const char* end_date);

/** 30. Fetch intraday OHLC for options. */
TdxOhlcTickArray tdx_option_history_ohlc(const TdxClient* client, const char* symbol, const char* expiration,
                                         const char* strike, const char* right,
                                         const char* date, const char* interval);

/** 31. Fetch all trades for an option. */
TdxTradeTickArray tdx_option_history_trade(const TdxClient* client, const char* symbol, const char* expiration,
                                           const char* strike, const char* right, const char* date);

/** 32. Fetch NBBO quotes for an option. */
TdxQuoteTickArray tdx_option_history_quote(const TdxClient* client, const char* symbol, const char* expiration,
                                           const char* strike, const char* right,
                                           const char* date, const char* interval);

/** 33. Fetch combined trade + quote for an option. */
TdxTradeQuoteTickArray tdx_option_history_trade_quote(const TdxClient* client, const char* symbol, const char* expiration,
                                                      const char* strike, const char* right, const char* date);

/** 34. Fetch open interest history. */
TdxOpenInterestTickArray tdx_option_history_open_interest(const TdxClient* client, const char* symbol, const char* expiration,
                                                          const char* strike, const char* right, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — History Greeks endpoints (11)                                */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 35-45: Greeks history endpoints all return TdxGreeksTickArray or TdxIvTickArray. */
TdxGreeksTickArray tdx_option_history_greeks_eod(const TdxClient* client, const char* symbol, const char* expiration,
                                                  const char* strike, const char* right,
                                                  const char* start_date, const char* end_date);

TdxGreeksTickArray tdx_option_history_greeks_all(const TdxClient* client, const char* symbol, const char* expiration,
                                                  const char* strike, const char* right,
                                                  const char* date, const char* interval);

TdxGreeksTickArray tdx_option_history_trade_greeks_all(const TdxClient* client, const char* symbol, const char* expiration,
                                                        const char* strike, const char* right, const char* date);

TdxGreeksTickArray tdx_option_history_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                          const char* strike, const char* right,
                                                          const char* date, const char* interval);

TdxGreeksTickArray tdx_option_history_trade_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                                const char* strike, const char* right, const char* date);

TdxGreeksTickArray tdx_option_history_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                           const char* strike, const char* right,
                                                           const char* date, const char* interval);

TdxGreeksTickArray tdx_option_history_trade_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                                 const char* strike, const char* right, const char* date);

TdxGreeksTickArray tdx_option_history_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                          const char* strike, const char* right,
                                                          const char* date, const char* interval);

TdxGreeksTickArray tdx_option_history_trade_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                                const char* strike, const char* right, const char* date);

TdxIvTickArray tdx_option_history_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration,
                                                             const char* strike, const char* right,
                                                             const char* date, const char* interval);

TdxIvTickArray tdx_option_history_trade_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration,
                                                                   const char* strike, const char* right, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — At-Time endpoints (2)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 46. Fetch trade at a specific time for an option. */
TdxTradeTickArray tdx_option_at_time_trade(const TdxClient* client, const char* symbol, const char* expiration,
                                           const char* strike, const char* right,
                                           const char* start_date, const char* end_date, const char* time_of_day);

/** 47. Fetch quote at a specific time for an option. */
TdxQuoteTickArray tdx_option_at_time_quote(const TdxClient* client, const char* symbol, const char* expiration,
                                           const char* strike, const char* right,
                                           const char* start_date, const char* end_date, const char* time_of_day);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — List endpoints (2)                                            */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 48. List all index symbols. */
TdxStringArray tdx_index_list_symbols(const TdxClient* client);

/** 49. List available dates for an index. */
TdxStringArray tdx_index_list_dates(const TdxClient* client, const char* symbol);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — Snapshot endpoints (3)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 50. Get latest OHLC snapshot for indices. symbols_json is JSON array. */
TdxOhlcTickArray tdx_index_snapshot_ohlc(const TdxClient* client, const char* symbols_json);

/** 51. Get latest price snapshot for indices. symbols_json is JSON array. */
TdxPriceTickArray tdx_index_snapshot_price(const TdxClient* client, const char* symbols_json);

/** 52. Get latest market value for indices. symbols_json is JSON array. */
TdxMarketValueTickArray tdx_index_snapshot_market_value(const TdxClient* client, const char* symbols_json);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — History endpoints (3)                                         */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 53. Fetch EOD index data. */
TdxEodTickArray tdx_index_history_eod(const TdxClient* client, const char* symbol,
                                      const char* start_date, const char* end_date);

/** 54. Fetch intraday OHLC for an index. */
TdxOhlcTickArray tdx_index_history_ohlc(const TdxClient* client, const char* symbol,
                                        const char* start_date, const char* end_date,
                                        const char* interval);

/** 55. Fetch intraday price history for an index. */
TdxPriceTickArray tdx_index_history_price(const TdxClient* client, const char* symbol,
                                          const char* date, const char* interval);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — At-Time endpoints (1)                                         */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 56. Fetch index price at a specific time. */
TdxPriceTickArray tdx_index_at_time_price(const TdxClient* client, const char* symbol,
                                          const char* start_date, const char* end_date,
                                          const char* time_of_day);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Calendar endpoints (3)                                                */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 57. Check whether the market is open today. */
TdxCalendarDayArray tdx_calendar_open_today(const TdxClient* client);

/** 58. Get calendar for a specific date. */
TdxCalendarDayArray tdx_calendar_on_date(const TdxClient* client, const char* date);

/** 59. Get calendar for an entire year. */
TdxCalendarDayArray tdx_calendar_year(const TdxClient* client, const char* year);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Interest Rate endpoints (1)                                           */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 60. Fetch EOD interest rate history. */
TdxInterestRateTickArray tdx_interest_rate_history_eod(const TdxClient* client, const char* symbol,
                                                       const char* start_date, const char* end_date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Greeks (standalone)                                                   */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Compute all 22 Greeks + IV. Returns JSON object. Caller must free with tdx_string_free. */
char* tdx_all_greeks(double spot, double strike, double rate, double div_yield,
                     double tte, double option_price, int is_call);

/** Compute implied volatility. Returns 0 on success, -1 on failure. */
int tdx_implied_volatility(double spot, double strike, double rate, double div_yield,
                           double tte, double option_price, int is_call,
                           double* out_iv, double* out_error);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  FPSS — #[repr(C)] streaming event types                               */
/* ═══════════════════════════════════════════════════════════════════════ */

/** FPSS event kind tag. Check this to determine which field of
 *  TdxFpssEvent is valid. */
typedef enum {
    TDX_FPSS_QUOTE = 0,
    TDX_FPSS_TRADE = 1,
    TDX_FPSS_OPEN_INTEREST = 2,
    TDX_FPSS_OHLCVC = 3,
    TDX_FPSS_CONTROL = 4,
    TDX_FPSS_RAW_DATA = 5,
} TdxFpssEventKind;

typedef struct {
    int32_t contract_id;
    int32_t ms_of_day;
    int32_t bid_size;
    int32_t bid_exchange;
    int32_t bid;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    int32_t ask;
    int32_t ask_condition;
    int32_t price_type;
    int32_t date;
    uint64_t received_at_ns;
} TdxFpssQuote;

typedef struct {
    int32_t contract_id;
    int32_t ms_of_day;
    int32_t sequence;
    int32_t ext_condition1;
    int32_t ext_condition2;
    int32_t ext_condition3;
    int32_t ext_condition4;
    int32_t condition;
    int32_t size;
    int32_t exchange;
    int32_t price;
    int32_t condition_flags;
    int32_t price_flags;
    int32_t volume_type;
    int32_t records_back;
    int32_t price_type;
    int32_t date;
    uint64_t received_at_ns;
} TdxFpssTrade;

typedef struct {
    int32_t contract_id;
    int32_t ms_of_day;
    int32_t open_interest;
    int32_t date;
    uint64_t received_at_ns;
} TdxFpssOpenInterest;

typedef struct {
    int32_t contract_id;
    int32_t ms_of_day;
    int32_t open;
    int32_t high;
    int32_t low;
    int32_t close;
    int64_t volume;
    int64_t count;
    int32_t price_type;
    int32_t date;
    uint64_t received_at_ns;
} TdxFpssOhlcvc;

/** FPSS control event.
 *  kind: 0=login_success, 1=contract_assigned, 2=req_response,
 *        3=market_open, 4=market_close, 5=server_error,
 *        6=disconnected, 7=error
 *  id:   contract_id or req_id where applicable, 0 otherwise.
 *  detail: NUL-terminated string, may be NULL. Do NOT free. */
typedef struct {
    int32_t kind;
    int32_t id;
    const char* detail;
} TdxFpssControl;

/** FPSS raw/undecoded data event. */
typedef struct {
    uint8_t code;
    const uint8_t* payload;
    size_t payload_len;
} TdxFpssRawData;

/** Tagged FPSS event. Check `kind` then read the corresponding field.
 *  Only the field matching `kind` contains valid data. */
typedef struct {
    TdxFpssEventKind kind;
    TdxFpssQuote quote;
    TdxFpssTrade trade;
    TdxFpssOpenInterest open_interest;
    TdxFpssOhlcvc ohlcvc;
    TdxFpssControl control;
    TdxFpssRawData raw_data;
} TdxFpssEvent;

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

/** Check if authenticated. Returns 1 if true, 0 if false. */
int tdx_fpss_is_authenticated(const TdxFpssHandle* h);

/** Look up a contract by server-assigned ID. Returns string or NULL. Caller must free with tdx_string_free. */
char* tdx_fpss_contract_lookup(const TdxFpssHandle* h, int id);

/** Get active subscriptions as JSON array. Caller must free with tdx_string_free. */
char* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);

/** Poll for the next event as a typed struct. Returns TdxFpssEvent* or NULL on timeout.
 *  Caller MUST free with tdx_fpss_event_free. */
TdxFpssEvent* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);

/** Free a TdxFpssEvent returned by tdx_fpss_next_event. */
void tdx_fpss_event_free(TdxFpssEvent* event);

/** Shut down the FPSS client. */
void tdx_fpss_shutdown(const TdxFpssHandle* h);

/** Free the FPSS handle. Must be called after tdx_fpss_shutdown. */
void tdx_fpss_free(TdxFpssHandle* h);

#ifdef __cplusplus
}
#endif

#endif /* THETADX_H */
