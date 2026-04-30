#ifndef THETADATADX_GO_FFI_BRIDGE_H
#define THETADATADX_GO_FFI_BRIDGE_H

#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

/* Opaque handles */
typedef struct TdxCredentials TdxCredentials;
typedef struct TdxClient TdxClient;
typedef struct TdxConfig TdxConfig;
typedef struct TdxFpssHandle TdxFpssHandle;
typedef struct TdxUnified TdxUnified;

/* Generic array layouts used by the Go bindings. */
typedef struct { const void* data; size_t len; } TdxTickArray;
typedef struct { const void* data; size_t len; } TdxStringArray;
typedef struct { const void* data; size_t len; } TdxOptionContractArray;

/* Generated request-options bridge shared with Rust FFI. */
#include "endpoint_request_options.h.inc"

/* Error */
extern const char* tdx_last_error(void);
extern void tdx_clear_error(void);
extern void tdx_string_free(char* s);

/* Credentials */
extern TdxCredentials* tdx_credentials_new(const char* email, const char* password);
extern TdxCredentials* tdx_credentials_from_file(const char* path);
extern void tdx_credentials_free(TdxCredentials* creds);

/* Config */
extern TdxConfig* tdx_config_production(void);
extern TdxConfig* tdx_config_dev(void);
extern TdxConfig* tdx_config_stage(void);
extern void tdx_config_free(TdxConfig* config);
extern void tdx_config_set_reconnect_policy(TdxConfig* config, int policy);
extern void tdx_config_set_flush_mode(TdxConfig* config, int mode);
extern void tdx_config_set_derive_ohlcvc(TdxConfig* config, int enabled);

/* Client */
extern TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);
extern void tdx_client_free(TdxClient* client);

/* Free functions */
extern void tdx_eod_tick_array_free(TdxTickArray arr);
extern void tdx_ohlc_tick_array_free(TdxTickArray arr);
extern void tdx_trade_tick_array_free(TdxTickArray arr);
extern void tdx_quote_tick_array_free(TdxTickArray arr);
extern void tdx_greeks_tick_array_free(TdxTickArray arr);
extern void tdx_iv_tick_array_free(TdxTickArray arr);
extern void tdx_price_tick_array_free(TdxTickArray arr);
extern void tdx_open_interest_tick_array_free(TdxTickArray arr);
extern void tdx_market_value_tick_array_free(TdxTickArray arr);
extern void tdx_calendar_day_array_free(TdxTickArray arr);
extern void tdx_interest_rate_tick_array_free(TdxTickArray arr);
extern void tdx_trade_quote_tick_array_free(TdxTickArray arr);
extern void tdx_option_contract_array_free(TdxOptionContractArray arr);
extern void tdx_string_array_free(TdxStringArray arr);

/* Historical endpoints */
extern TdxStringArray tdx_stock_list_symbols(const TdxClient* client);
extern TdxStringArray tdx_stock_list_dates(const TdxClient* client, const char* request_type, const char* symbol);
extern TdxTickArray tdx_stock_snapshot_ohlc(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_snapshot_trade(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_snapshot_quote(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_snapshot_market_value(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern TdxTickArray tdx_stock_history_ohlc(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern TdxTickArray tdx_stock_history_ohlc_range(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern TdxTickArray tdx_stock_history_trade(const TdxClient* client, const char* symbol, const char* date);
extern TdxTickArray tdx_stock_history_quote(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern TdxTickArray tdx_stock_history_trade_quote(const TdxClient* client, const char* symbol, const char* date);
extern TdxTickArray tdx_stock_at_time_trade(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxTickArray tdx_stock_at_time_quote(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxStringArray tdx_option_list_symbols(const TdxClient* client);
extern TdxStringArray tdx_option_list_dates(const TdxClient* client, const char* request_type, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxStringArray tdx_option_list_expirations(const TdxClient* client, const char* symbol);
extern TdxStringArray tdx_option_list_strikes(const TdxClient* client, const char* symbol, const char* expiration);
extern TdxOptionContractArray tdx_option_list_contracts(const TdxClient* client, const char* request_type, const char* symbol, const char* date);
extern TdxTickArray tdx_option_snapshot_ohlc(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_open_interest(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_market_value(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_greeks_all(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_snapshot_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxTickArray tdx_option_history_eod(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date);
extern TdxTickArray tdx_option_history_ohlc(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_open_interest(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_greeks_eod(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date);
extern TdxTickArray tdx_option_history_greeks_all(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_greeks_all(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_at_time_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxTickArray tdx_option_at_time_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxStringArray tdx_index_list_symbols(const TdxClient* client);
extern TdxStringArray tdx_index_list_dates(const TdxClient* client, const char* symbol);
extern TdxTickArray tdx_index_snapshot_ohlc(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_index_snapshot_price(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_index_snapshot_market_value(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_index_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern TdxTickArray tdx_index_history_ohlc(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern TdxTickArray tdx_index_history_price(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern TdxTickArray tdx_index_at_time_price(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxTickArray tdx_calendar_open_today(const TdxClient* client);
extern TdxTickArray tdx_calendar_on_date(const TdxClient* client, const char* date);
extern TdxTickArray tdx_calendar_year(const TdxClient* client, const char* year);
extern TdxTickArray tdx_interest_rate_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
/* Generated option-aware endpoint declarations. */
#include "endpoint_with_options.h.inc"

/* Greeks */
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

extern TdxGreeksResult* tdx_all_greeks(double spot, double strike, double rate, double div_yield, double tte, double option_price, const char* right);
extern void tdx_greeks_result_free(TdxGreeksResult* result);
extern int tdx_implied_volatility(double spot, double strike, double rate, double div_yield, double tte, double option_price, const char* right, double* out_iv, double* out_error);

/* FPSS subscription metadata */
typedef struct {
    const char* kind;
    const char* contract;
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

extern void tdx_subscription_array_free(TdxSubscriptionArray* arr);
extern void tdx_contract_map_array_free(TdxContractMapArray* arr);

/* FPSS events — generated from fpss_event_schema.toml */
#include "fpss_event_structs.h.inc"

/* FPSS client */
extern TdxFpssHandle* tdx_fpss_connect(const TdxCredentials* creds, const TdxConfig* config);
extern int tdx_fpss_subscribe_quotes(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_subscribe_trades(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_subscribe_open_interest(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_subscribe_full_trades(const TdxFpssHandle* h, const char* sec_type);
extern int tdx_fpss_subscribe_full_open_interest(const TdxFpssHandle* h, const char* sec_type);
extern int tdx_fpss_unsubscribe_quotes(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_unsubscribe_trades(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_unsubscribe_open_interest(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_unsubscribe_full_trades(const TdxFpssHandle* h, const char* sec_type);
extern int tdx_fpss_unsubscribe_full_open_interest(const TdxFpssHandle* h, const char* sec_type);
extern int tdx_fpss_subscribe_option_quotes(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_fpss_subscribe_option_trades(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_fpss_subscribe_option_open_interest(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_fpss_unsubscribe_option_quotes(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_fpss_unsubscribe_option_trades(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_fpss_unsubscribe_option_open_interest(const TdxFpssHandle* h, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_fpss_is_authenticated(const TdxFpssHandle* h);
/* Look up a contract by server-assigned ID. Returns string or NULL.
 * NULL with empty tdx_last_error() means "not found". NULL with non-empty
 * tdx_last_error() means a real error occurred. Caller must free with tdx_string_free. */
extern char* tdx_fpss_contract_lookup(const TdxFpssHandle* h, int id);
extern TdxContractMapArray* tdx_fpss_contract_map(const TdxFpssHandle* h);
extern TdxSubscriptionArray* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);
extern TdxFpssEvent* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);
extern void tdx_fpss_event_free(TdxFpssEvent* event);
extern int tdx_fpss_reconnect(const TdxFpssHandle* h);
/* Cumulative count of FPSS events dropped because the internal receiver
 * was gone when the callback tried to deliver. Survives reconnect. Returns
 * 0 if the handle is null. */
extern uint64_t tdx_fpss_dropped_events(const TdxFpssHandle* h);
extern void tdx_fpss_shutdown(const TdxFpssHandle* h);
extern void tdx_fpss_free(TdxFpssHandle* h);

/* Unified client -- historical + streaming through one handle */
extern TdxUnified* tdx_unified_connect(const TdxCredentials* creds, const TdxConfig* config);
extern int tdx_unified_start_streaming(const TdxUnified* handle);
extern int tdx_unified_subscribe_quotes(const TdxUnified* handle, const char* symbol);
extern int tdx_unified_subscribe_trades(const TdxUnified* handle, const char* symbol);
extern int tdx_unified_unsubscribe_quotes(const TdxUnified* handle, const char* symbol);
extern int tdx_unified_unsubscribe_trades(const TdxUnified* handle, const char* symbol);
extern int tdx_unified_subscribe_open_interest(const TdxUnified* handle, const char* symbol);
extern int tdx_unified_unsubscribe_open_interest(const TdxUnified* handle, const char* symbol);
extern int tdx_unified_subscribe_full_trades(const TdxUnified* handle, const char* sec_type);
extern int tdx_unified_subscribe_full_open_interest(const TdxUnified* handle, const char* sec_type);
extern int tdx_unified_unsubscribe_full_trades(const TdxUnified* handle, const char* sec_type);
extern int tdx_unified_unsubscribe_full_open_interest(const TdxUnified* handle, const char* sec_type);
extern int tdx_unified_subscribe_option_quotes(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_unified_subscribe_option_trades(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_unified_subscribe_option_open_interest(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_unified_unsubscribe_option_quotes(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_unified_unsubscribe_option_trades(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);
extern int tdx_unified_unsubscribe_option_open_interest(const TdxUnified* handle, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxContractMapArray* tdx_unified_contract_map(const TdxUnified* handle);
extern int tdx_unified_reconnect(const TdxUnified* handle);
extern int tdx_unified_is_streaming(const TdxUnified* handle);
/* Look up a contract by ID. Returns string or NULL.
 * NULL with empty tdx_last_error() means "not found". NULL with non-empty
 * tdx_last_error() means a real error occurred. Caller must free with tdx_string_free. */
extern char* tdx_unified_contract_lookup(const TdxUnified* handle, int id);
extern TdxSubscriptionArray* tdx_unified_active_subscriptions(const TdxUnified* handle);
extern TdxFpssEvent* tdx_unified_next_event(const TdxUnified* handle, uint64_t timeout_ms);
extern const TdxClient* tdx_unified_historical(const TdxUnified* handle);
extern void tdx_unified_stop_streaming(const TdxUnified* handle);
/* Cumulative count of FPSS events dropped because the internal receiver
 * was gone when the callback tried to deliver. Survives reconnect. Returns
 * 0 if the handle is null. */
extern uint64_t tdx_unified_dropped_events(const TdxUnified* handle);
extern void tdx_unified_free(TdxUnified* handle);

#endif
