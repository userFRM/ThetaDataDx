package thetadatadx

/*
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -lthetadatadx_ffi -lm -ldl -lpthread
#cgo darwin LDFLAGS: -framework Security -framework SystemConfiguration
#include <stdlib.h>
#include <stdint.h>
#include <stddef.h>

// ── Opaque handles ──
typedef void TdxCredentials;
typedef void TdxClient;
typedef void TdxConfig;
typedef void TdxFpssHandle;

// ── Error ──
extern const char* tdx_last_error();

// ── Credentials ──
extern TdxCredentials* tdx_credentials_new(const char* email, const char* password);
extern TdxCredentials* tdx_credentials_from_file(const char* path);
extern void tdx_credentials_free(TdxCredentials* creds);

// ── Config ──
extern TdxConfig* tdx_config_production();
extern TdxConfig* tdx_config_dev();
extern TdxConfig* tdx_config_stage();
extern void tdx_config_free(TdxConfig* config);

// ── Client ──
extern TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);
extern void tdx_client_free(TdxClient* client);

// ── String free ──
extern void tdx_string_free(char* s);

// ── Typed array types ──
// Each has {data, len} where data is a pointer to #[repr(C)] structs.
// We use void* for data and size_t for len, then cast on the Go side.

typedef struct { const void* data; size_t len; } TdxTickArray;
typedef struct { const void* data; size_t len; } TdxStringArray;
typedef struct { const void* data; size_t len; } TdxOptionContractArray;

// ── Free functions for typed arrays ──
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
extern void tdx_snapshot_trade_tick_array_free(TdxTickArray arr);
extern void tdx_trade_quote_tick_array_free(TdxTickArray arr);
extern void tdx_option_contract_array_free(TdxOptionContractArray arr);
extern void tdx_string_array_free(TdxStringArray arr);

// ── Stock — List endpoints ──
extern TdxStringArray tdx_stock_list_symbols(const TdxClient* client);
extern TdxStringArray tdx_stock_list_dates(const TdxClient* client, const char* request_type, const char* symbol);

// ── Stock — Snapshot endpoints ──
extern TdxTickArray tdx_stock_snapshot_ohlc(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_snapshot_trade(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_snapshot_quote(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_stock_snapshot_market_value(const TdxClient* client, const char* const* symbols, size_t symbols_len);

// ── Stock — History endpoints ──
extern TdxTickArray tdx_stock_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern TdxTickArray tdx_stock_history_ohlc(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern TdxTickArray tdx_stock_history_ohlc_range(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern TdxTickArray tdx_stock_history_trade(const TdxClient* client, const char* symbol, const char* date);
extern TdxTickArray tdx_stock_history_quote(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern TdxTickArray tdx_stock_history_trade_quote(const TdxClient* client, const char* symbol, const char* date);

// ── Stock — At-Time endpoints ──
extern TdxTickArray tdx_stock_at_time_trade(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxTickArray tdx_stock_at_time_quote(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);

// ── Option — List endpoints ──
extern TdxStringArray tdx_option_list_symbols(const TdxClient* client);
extern TdxStringArray tdx_option_list_dates(const TdxClient* client, const char* request_type, const char* symbol, const char* expiration, const char* strike, const char* right);
extern TdxStringArray tdx_option_list_expirations(const TdxClient* client, const char* symbol);
extern TdxStringArray tdx_option_list_strikes(const TdxClient* client, const char* symbol, const char* expiration);
extern TdxOptionContractArray tdx_option_list_contracts(const TdxClient* client, const char* request_type, const char* symbol, const char* date);

// ── Option — Snapshot endpoints ──
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

// ── Option — History endpoints ──
extern TdxTickArray tdx_option_history_eod(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date);
extern TdxTickArray tdx_option_history_ohlc(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern TdxTickArray tdx_option_history_trade_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern TdxTickArray tdx_option_history_open_interest(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);

// ── Option — History Greeks endpoints ──
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

// ── Option — At-Time endpoints ──
extern TdxTickArray tdx_option_at_time_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);
extern TdxTickArray tdx_option_at_time_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);

// ── Index ──
extern TdxStringArray tdx_index_list_symbols(const TdxClient* client);
extern TdxStringArray tdx_index_list_dates(const TdxClient* client, const char* symbol);
extern TdxTickArray tdx_index_snapshot_ohlc(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_index_snapshot_price(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_index_snapshot_market_value(const TdxClient* client, const char* const* symbols, size_t symbols_len);
extern TdxTickArray tdx_index_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern TdxTickArray tdx_index_history_ohlc(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern TdxTickArray tdx_index_history_price(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern TdxTickArray tdx_index_at_time_price(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);

// ── Calendar ──
extern TdxTickArray tdx_calendar_open_today(const TdxClient* client);
extern TdxTickArray tdx_calendar_on_date(const TdxClient* client, const char* date);
extern TdxTickArray tdx_calendar_year(const TdxClient* client, const char* year);

// ── Interest Rate ──
extern TdxTickArray tdx_interest_rate_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);

// ── Greeks result struct ──
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

// ── Subscription types ──
typedef struct {
    const char* kind;
    const char* contract;
} TdxSubscription;

typedef struct {
    const TdxSubscription* data;
    size_t len;
} TdxSubscriptionArray;

// ── Greeks ──
extern TdxGreeksResult* tdx_all_greeks(double spot, double strike, double rate, double div_yield, double tte, double option_price, int is_call);
extern void tdx_greeks_result_free(TdxGreeksResult* result);
extern int tdx_implied_volatility(double spot, double strike, double rate, double div_yield, double tte, double option_price, int is_call, double* out_iv, double* out_error);
extern void tdx_subscription_array_free(TdxSubscriptionArray* arr);

// ── FPSS (real-time streaming) ──
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
extern int tdx_fpss_is_authenticated(const TdxFpssHandle* h);
extern char* tdx_fpss_contract_lookup(const TdxFpssHandle* h, int id);
extern char* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);
extern void tdx_fpss_shutdown(const TdxFpssHandle* h);
extern void tdx_fpss_free(TdxFpssHandle* h);
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// lastError returns the most recent FFI error string.
func lastError() string {
	p := C.tdx_last_error()
	if p == nil {
		return "unknown error"
	}
	return C.GoString(p)
}

// stringArrayToGo converts a TdxStringArray to a Go []string and frees the C memory.
func stringArrayToGo(arr C.TdxStringArray) ([]string, error) {
	if arr.data == nil || arr.len == 0 {
		C.tdx_string_array_free(arr)
		return nil, nil
	}
	n := int(arr.len)
	// Create a Go slice backed by the C array of char* pointers.
	ptrs := unsafe.Slice((**C.char)(arr.data), n)
	result := make([]string, n)
	for i := 0; i < n; i++ {
		if ptrs[i] != nil {
			result[i] = C.GoString(ptrs[i])
		}
	}
	C.tdx_string_array_free(arr)
	return result, nil
}

// ── Credentials ──

// Credentials holds ThetaData authentication credentials.
type Credentials struct {
	handle *C.TdxCredentials
}

// NewCredentials creates credentials from email and password.
func NewCredentials(email, password string) (*Credentials, error) {
	cEmail := C.CString(email)
	cPassword := C.CString(password)
	defer C.free(unsafe.Pointer(cEmail))
	defer C.free(unsafe.Pointer(cPassword))

	h := C.tdx_credentials_new(cEmail, cPassword)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &Credentials{handle: h}, nil
}

// CredentialsFromFile loads credentials from a file (line 1 = email, line 2 = password).
func CredentialsFromFile(path string) (*Credentials, error) {
	cPath := C.CString(path)
	defer C.free(unsafe.Pointer(cPath))

	h := C.tdx_credentials_from_file(cPath)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &Credentials{handle: h}, nil
}

// Close frees the credentials handle.
func (c *Credentials) Close() {
	if c.handle != nil {
		C.tdx_credentials_free(c.handle)
		c.handle = nil
	}
}

// ── Config ──

// Config holds connection configuration.
type Config struct {
	handle *C.TdxConfig
}

// ProductionConfig returns the production server config (ThetaData NJ datacenter).
func ProductionConfig() *Config {
	return &Config{handle: C.tdx_config_production()}
}

// DevConfig returns the dev FPSS config (port 20200, infinite historical replay).
func DevConfig() *Config {
	return &Config{handle: C.tdx_config_dev()}
}

// StageConfig returns the stage FPSS config (port 20100, testing, unstable).
func StageConfig() *Config {
	return &Config{handle: C.tdx_config_stage()}
}

// Close frees the config handle.
func (c *Config) Close() {
	if c.handle != nil {
		C.tdx_config_free(c.handle)
		c.handle = nil
	}
}

// symbolsToCArray converts a Go string slice into a C array of C strings.
// The caller must free each element and the array itself with C.free.
func symbolsToCArray(symbols []string) (**C.char, C.size_t) {
	n := len(symbols)
	if n == 0 {
		return nil, 0
	}
	// Allocate an array of *C.char pointers.
	cArray := C.malloc(C.size_t(n) * C.size_t(unsafe.Sizeof((*C.char)(nil))))
	ptrs := (*[1 << 30]*C.char)(cArray)
	for i, s := range symbols {
		ptrs[i] = C.CString(s)
	}
	return (**C.char)(cArray), C.size_t(n)
}

// freeSymbolArray frees a C array of C strings allocated by symbolsToCArray.
func freeSymbolArray(arr **C.char, n C.size_t) {
	if arr == nil {
		return
	}
	ptrs := (*[1 << 30]*C.char)(unsafe.Pointer(arr))
	for i := C.size_t(0); i < n; i++ {
		C.free(unsafe.Pointer(ptrs[i]))
	}
	C.free(unsafe.Pointer(arr))
}
