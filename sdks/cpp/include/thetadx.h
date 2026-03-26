/**
 * thetadatadx C FFI header.
 *
 * This header declares the C interface to the thetadatadx Rust SDK.
 * Used by both the C++ wrapper and any other C-compatible language.
 *
 * Memory model:
 * - Opaque handles (TdxCredentials*, TdxClient*, TdxConfig*) are heap-allocated
 *   by the Rust side and MUST be freed with the corresponding tdx_*_free function.
 * - String results (char*) are heap-allocated JSON and MUST be freed with tdx_string_free.
 * - Functions that can fail return NULL and set a thread-local error string
 *   retrievable via tdx_last_error().
 */

#ifndef THETADX_H
#define THETADX_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Opaque handle types ── */
typedef struct TdxCredentials TdxCredentials;
typedef struct TdxClient TdxClient;
typedef struct TdxConfig TdxConfig;
typedef struct TdxFpssHandle TdxFpssHandle;

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

/** Create a dev config (shorter timeouts). */
TdxConfig* tdx_config_dev(void);

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

/** 1. List all stock symbols. Returns JSON array. */
char* tdx_stock_list_symbols(const TdxClient* client);

/** 2. List available dates for a stock by request type. Returns JSON array. */
char* tdx_stock_list_dates(const TdxClient* client, const char* request_type, const char* symbol);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — Snapshot endpoints (4)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 3. Get latest OHLC snapshot. symbols_json is JSON array. Returns JSON array. */
char* tdx_stock_snapshot_ohlc(const TdxClient* client, const char* symbols_json);

/** 4. Get latest trade snapshot. symbols_json is JSON array. Returns JSON array. */
char* tdx_stock_snapshot_trade(const TdxClient* client, const char* symbols_json);

/** 5. Get latest NBBO quote snapshot. symbols_json is JSON array. Returns JSON array. */
char* tdx_stock_snapshot_quote(const TdxClient* client, const char* symbols_json);

/** 6. Get latest market value snapshot. symbols_json is JSON array. Returns JSON DataTable. */
char* tdx_stock_snapshot_market_value(const TdxClient* client, const char* symbols_json);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — History endpoints (5 + bonus)                                 */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 7. Fetch stock EOD history. Returns JSON array. */
char* tdx_stock_history_eod(const TdxClient* client, const char* symbol,
                            const char* start_date, const char* end_date);

/** 8. Fetch stock intraday OHLC. Returns JSON array. */
char* tdx_stock_history_ohlc(const TdxClient* client, const char* symbol,
                             const char* date, const char* interval);

/** 8b. Fetch stock OHLC across date range. Returns JSON array. */
char* tdx_stock_history_ohlc_range(const TdxClient* client, const char* symbol,
                                   const char* start_date, const char* end_date,
                                   const char* interval);

/** 9. Fetch all trades on a date. Returns JSON array. */
char* tdx_stock_history_trade(const TdxClient* client, const char* symbol, const char* date);

/** 10. Fetch NBBO quotes. Returns JSON array. */
char* tdx_stock_history_quote(const TdxClient* client, const char* symbol,
                              const char* date, const char* interval);

/** 11. Fetch combined trade + quote ticks. Returns JSON DataTable. */
char* tdx_stock_history_trade_quote(const TdxClient* client, const char* symbol, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Stock — At-Time endpoints (2)                                         */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 12. Fetch trade at a specific time across a date range. Returns JSON array. */
char* tdx_stock_at_time_trade(const TdxClient* client, const char* symbol,
                              const char* start_date, const char* end_date,
                              const char* time_of_day);

/** 13. Fetch quote at a specific time across a date range. Returns JSON array. */
char* tdx_stock_at_time_quote(const TdxClient* client, const char* symbol,
                              const char* start_date, const char* end_date,
                              const char* time_of_day);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — List endpoints (5)                                           */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 14. List all option underlyings. Returns JSON array. */
char* tdx_option_list_symbols(const TdxClient* client);

/** 15. List available dates for an option contract. Returns JSON array. */
char* tdx_option_list_dates(const TdxClient* client, const char* request_type,
                            const char* symbol, const char* expiration,
                            const char* strike, const char* right);

/** 16. List expiration dates. Returns JSON array. */
char* tdx_option_list_expirations(const TdxClient* client, const char* symbol);

/** 17. List strike prices. Returns JSON array. */
char* tdx_option_list_strikes(const TdxClient* client, const char* symbol,
                              const char* expiration);

/** 18. List all option contracts on a date. Returns JSON DataTable. */
char* tdx_option_list_contracts(const TdxClient* client, const char* request_type,
                                const char* symbol, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — Snapshot endpoints (10)                                      */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 19. Get latest OHLC snapshot for options. Returns JSON array. */
char* tdx_option_snapshot_ohlc(const TdxClient* client, const char* symbol,
                               const char* expiration, const char* strike, const char* right);

/** 20. Get latest trade snapshot for options. Returns JSON array. */
char* tdx_option_snapshot_trade(const TdxClient* client, const char* symbol,
                                const char* expiration, const char* strike, const char* right);

/** 21. Get latest NBBO quote snapshot for options. Returns JSON array. */
char* tdx_option_snapshot_quote(const TdxClient* client, const char* symbol,
                                const char* expiration, const char* strike, const char* right);

/** 22. Get latest open interest snapshot. Returns JSON DataTable. */
char* tdx_option_snapshot_open_interest(const TdxClient* client, const char* symbol,
                                        const char* expiration, const char* strike, const char* right);

/** 23. Get latest market value snapshot for options. Returns JSON DataTable. */
char* tdx_option_snapshot_market_value(const TdxClient* client, const char* symbol,
                                       const char* expiration, const char* strike, const char* right);

/** 24. Get IV snapshot for options. Returns JSON DataTable. */
char* tdx_option_snapshot_greeks_implied_volatility(const TdxClient* client, const char* symbol,
                                                     const char* expiration, const char* strike, const char* right);

/** 25. Get all Greeks snapshot. Returns JSON DataTable. */
char* tdx_option_snapshot_greeks_all(const TdxClient* client, const char* symbol,
                                     const char* expiration, const char* strike, const char* right);

/** 26. Get first-order Greeks snapshot. Returns JSON DataTable. */
char* tdx_option_snapshot_greeks_first_order(const TdxClient* client, const char* symbol,
                                              const char* expiration, const char* strike, const char* right);

/** 27. Get second-order Greeks snapshot. Returns JSON DataTable. */
char* tdx_option_snapshot_greeks_second_order(const TdxClient* client, const char* symbol,
                                               const char* expiration, const char* strike, const char* right);

/** 28. Get third-order Greeks snapshot. Returns JSON DataTable. */
char* tdx_option_snapshot_greeks_third_order(const TdxClient* client, const char* symbol,
                                              const char* expiration, const char* strike, const char* right);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — History endpoints (6)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 29. Fetch EOD option data. Returns JSON array. */
char* tdx_option_history_eod(const TdxClient* client, const char* symbol, const char* expiration,
                             const char* strike, const char* right,
                             const char* start_date, const char* end_date);

/** 30. Fetch intraday OHLC for options. Returns JSON array. */
char* tdx_option_history_ohlc(const TdxClient* client, const char* symbol, const char* expiration,
                              const char* strike, const char* right,
                              const char* date, const char* interval);

/** 31. Fetch all trades for an option. Returns JSON array. */
char* tdx_option_history_trade(const TdxClient* client, const char* symbol, const char* expiration,
                               const char* strike, const char* right, const char* date);

/** 32. Fetch NBBO quotes for an option. Returns JSON array. */
char* tdx_option_history_quote(const TdxClient* client, const char* symbol, const char* expiration,
                               const char* strike, const char* right,
                               const char* date, const char* interval);

/** 33. Fetch combined trade + quote for an option. Returns JSON DataTable. */
char* tdx_option_history_trade_quote(const TdxClient* client, const char* symbol, const char* expiration,
                                     const char* strike, const char* right, const char* date);

/** 34. Fetch open interest history. Returns JSON DataTable. */
char* tdx_option_history_open_interest(const TdxClient* client, const char* symbol, const char* expiration,
                                       const char* strike, const char* right, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — History Greeks endpoints (11)                                */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 35. Fetch EOD Greeks history. Returns JSON DataTable. */
char* tdx_option_history_greeks_eod(const TdxClient* client, const char* symbol, const char* expiration,
                                    const char* strike, const char* right,
                                    const char* start_date, const char* end_date);

/** 36. Fetch all Greeks history (intraday). Returns JSON DataTable. */
char* tdx_option_history_greeks_all(const TdxClient* client, const char* symbol, const char* expiration,
                                    const char* strike, const char* right,
                                    const char* date, const char* interval);

/** 37. Fetch all Greeks on each trade. Returns JSON DataTable. */
char* tdx_option_history_trade_greeks_all(const TdxClient* client, const char* symbol, const char* expiration,
                                          const char* strike, const char* right, const char* date);

/** 38. Fetch first-order Greeks history. Returns JSON DataTable. */
char* tdx_option_history_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration,
                                            const char* strike, const char* right,
                                            const char* date, const char* interval);

/** 39. Fetch first-order Greeks on each trade. Returns JSON DataTable. */
char* tdx_option_history_trade_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                   const char* strike, const char* right, const char* date);

/** 40. Fetch second-order Greeks history. Returns JSON DataTable. */
char* tdx_option_history_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration,
                                             const char* strike, const char* right,
                                             const char* date, const char* interval);

/** 41. Fetch second-order Greeks on each trade. Returns JSON DataTable. */
char* tdx_option_history_trade_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                    const char* strike, const char* right, const char* date);

/** 42. Fetch third-order Greeks history. Returns JSON DataTable. */
char* tdx_option_history_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration,
                                            const char* strike, const char* right,
                                            const char* date, const char* interval);

/** 43. Fetch third-order Greeks on each trade. Returns JSON DataTable. */
char* tdx_option_history_trade_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration,
                                                   const char* strike, const char* right, const char* date);

/** 44. Fetch IV history (intraday). Returns JSON DataTable. */
char* tdx_option_history_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration,
                                                    const char* strike, const char* right,
                                                    const char* date, const char* interval);

/** 45. Fetch IV on each trade. Returns JSON DataTable. */
char* tdx_option_history_trade_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration,
                                                          const char* strike, const char* right, const char* date);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Option — At-Time endpoints (2)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 46. Fetch trade at a specific time for an option. Returns JSON array. */
char* tdx_option_at_time_trade(const TdxClient* client, const char* symbol, const char* expiration,
                               const char* strike, const char* right,
                               const char* start_date, const char* end_date, const char* time_of_day);

/** 47. Fetch quote at a specific time for an option. Returns JSON array. */
char* tdx_option_at_time_quote(const TdxClient* client, const char* symbol, const char* expiration,
                               const char* strike, const char* right,
                               const char* start_date, const char* end_date, const char* time_of_day);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — List endpoints (2)                                            */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 48. List all index symbols. Returns JSON array. */
char* tdx_index_list_symbols(const TdxClient* client);

/** 49. List available dates for an index. Returns JSON array. */
char* tdx_index_list_dates(const TdxClient* client, const char* symbol);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — Snapshot endpoints (3)                                        */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 50. Get latest OHLC snapshot for indices. symbols_json is JSON array. Returns JSON array. */
char* tdx_index_snapshot_ohlc(const TdxClient* client, const char* symbols_json);

/** 51. Get latest price snapshot for indices. symbols_json is JSON array. Returns JSON DataTable. */
char* tdx_index_snapshot_price(const TdxClient* client, const char* symbols_json);

/** 52. Get latest market value for indices. symbols_json is JSON array. Returns JSON DataTable. */
char* tdx_index_snapshot_market_value(const TdxClient* client, const char* symbols_json);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — History endpoints (3)                                         */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 53. Fetch EOD index data. Returns JSON array. */
char* tdx_index_history_eod(const TdxClient* client, const char* symbol,
                            const char* start_date, const char* end_date);

/** 54. Fetch intraday OHLC for an index. Returns JSON array. */
char* tdx_index_history_ohlc(const TdxClient* client, const char* symbol,
                             const char* start_date, const char* end_date,
                             const char* interval);

/** 55. Fetch intraday price history for an index. Returns JSON DataTable. */
char* tdx_index_history_price(const TdxClient* client, const char* symbol,
                              const char* date, const char* interval);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Index — At-Time endpoints (1)                                         */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 56. Fetch index price at a specific time. Returns JSON DataTable. */
char* tdx_index_at_time_price(const TdxClient* client, const char* symbol,
                              const char* start_date, const char* end_date,
                              const char* time_of_day);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Calendar endpoints (3)                                                */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 57. Check whether the market is open today. Returns JSON DataTable. */
char* tdx_calendar_open_today(const TdxClient* client);

/** 58. Get calendar for a specific date. Returns JSON DataTable. */
char* tdx_calendar_on_date(const TdxClient* client, const char* date);

/** 59. Get calendar for an entire year. Returns JSON DataTable. */
char* tdx_calendar_year(const TdxClient* client, const char* year);

/* ═══════════════════════════════════════════════════════════════════════ */
/*  Interest Rate endpoints (1)                                           */
/* ═══════════════════════════════════════════════════════════════════════ */

/** 60. Fetch EOD interest rate history. Returns JSON DataTable. */
char* tdx_interest_rate_history_eod(const TdxClient* client, const char* symbol,
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
/*  FPSS — Real-time streaming client                                     */
/* ═══════════════════════════════════════════════════════════════════════ */

/** Connect to FPSS streaming servers. Returns NULL on failure. */
TdxFpssHandle* tdx_fpss_connect(const TdxCredentials* creds, const TdxConfig* config);

/** Subscribe to quote data. Returns request ID or -1 on error. */
int tdx_fpss_subscribe_quotes(const TdxFpssHandle* h, const char* symbol);

/** Subscribe to trade data. Returns request ID or -1 on error. */
int tdx_fpss_subscribe_trades(const TdxFpssHandle* h, const char* symbol);

/** Unsubscribe from quote data. Returns request ID or -1 on error. */
int tdx_fpss_unsubscribe_quotes(const TdxFpssHandle* h, const char* symbol);

/** Poll for the next event. Returns JSON string or NULL on timeout. Caller must free with tdx_string_free. */
char* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);

/** Shut down the FPSS client. */
void tdx_fpss_shutdown(const TdxFpssHandle* h);

/** Free the FPSS handle. Must be called after tdx_fpss_shutdown. */
void tdx_fpss_free(TdxFpssHandle* h);

#ifdef __cplusplus
}
#endif

#endif /* THETADX_H */
