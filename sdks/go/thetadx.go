package thetadatadx

/*
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -lthetadatadx_ffi -lm -ldl -lpthread
#cgo darwin LDFLAGS: -framework Security -framework SystemConfiguration
#include <stdlib.h>

// ── Opaque handles ──
typedef void TdxCredentials;
typedef void TdxClient;
typedef void TdxConfig;

// ── Error ──
extern const char* tdx_last_error();

// ── Credentials ──
extern TdxCredentials* tdx_credentials_new(const char* email, const char* password);
extern TdxCredentials* tdx_credentials_from_file(const char* path);
extern void tdx_credentials_free(TdxCredentials* creds);

// ── Config ──
extern TdxConfig* tdx_config_production();
extern TdxConfig* tdx_config_dev();
extern void tdx_config_free(TdxConfig* config);

// ── Client ──
extern TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);
extern void tdx_client_free(TdxClient* client);

// ── String free ──
extern void tdx_string_free(char* s);

// ── Stock — List endpoints (2) ──
extern char* tdx_stock_list_symbols(const TdxClient* client);
extern char* tdx_stock_list_dates(const TdxClient* client, const char* request_type, const char* symbol);

// ── Stock — Snapshot endpoints (4) ──
extern char* tdx_stock_snapshot_ohlc(const TdxClient* client, const char* symbols_json);
extern char* tdx_stock_snapshot_trade(const TdxClient* client, const char* symbols_json);
extern char* tdx_stock_snapshot_quote(const TdxClient* client, const char* symbols_json);
extern char* tdx_stock_snapshot_market_value(const TdxClient* client, const char* symbols_json);

// ── Stock — History endpoints (5 + bonus) ──
extern char* tdx_stock_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern char* tdx_stock_history_ohlc(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern char* tdx_stock_history_ohlc_range(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern char* tdx_stock_history_trade(const TdxClient* client, const char* symbol, const char* date);
extern char* tdx_stock_history_quote(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern char* tdx_stock_history_trade_quote(const TdxClient* client, const char* symbol, const char* date);

// ── Stock — At-Time endpoints (2) ──
extern char* tdx_stock_at_time_trade(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);
extern char* tdx_stock_at_time_quote(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);

// ── Option — List endpoints (5) ──
extern char* tdx_option_list_symbols(const TdxClient* client);
extern char* tdx_option_list_dates(const TdxClient* client, const char* request_type, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_list_expirations(const TdxClient* client, const char* symbol);
extern char* tdx_option_list_strikes(const TdxClient* client, const char* symbol, const char* expiration);
extern char* tdx_option_list_contracts(const TdxClient* client, const char* request_type, const char* symbol, const char* date);

// ── Option — Snapshot endpoints (10) ──
extern char* tdx_option_snapshot_ohlc(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_open_interest(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_market_value(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_greeks_all(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_snapshot_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right);

// ── Option — History endpoints (6) ──
extern char* tdx_option_history_eod(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date);
extern char* tdx_option_history_ohlc(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_open_interest(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);

// ── Option — History Greeks endpoints (11) ──
extern char* tdx_option_history_greeks_eod(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date);
extern char* tdx_option_history_greeks_all(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_greeks_all(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_greeks_first_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_greeks_second_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_greeks_third_order(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_greeks_implied_volatility(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);

// ── Option — At-Time endpoints (2) ──
extern char* tdx_option_at_time_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);
extern char* tdx_option_at_time_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);

// ── Index — List endpoints (2) ──
extern char* tdx_index_list_symbols(const TdxClient* client);
extern char* tdx_index_list_dates(const TdxClient* client, const char* symbol);

// ── Index — Snapshot endpoints (3) ──
extern char* tdx_index_snapshot_ohlc(const TdxClient* client, const char* symbols_json);
extern char* tdx_index_snapshot_price(const TdxClient* client, const char* symbols_json);
extern char* tdx_index_snapshot_market_value(const TdxClient* client, const char* symbols_json);

// ── Index — History endpoints (3) ──
extern char* tdx_index_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern char* tdx_index_history_ohlc(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern char* tdx_index_history_price(const TdxClient* client, const char* symbol, const char* date, const char* interval);

// ── Index — At-Time endpoints (1) ──
extern char* tdx_index_at_time_price(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);

// ── Calendar endpoints (3) ──
extern char* tdx_calendar_open_today(const TdxClient* client);
extern char* tdx_calendar_on_date(const TdxClient* client, const char* date);
extern char* tdx_calendar_year(const TdxClient* client, const char* year);

// ── Interest Rate endpoints (1) ──
extern char* tdx_interest_rate_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);

// ── Greeks ──
extern char* tdx_all_greeks(double spot, double strike, double rate, double div_yield, double tte, double option_price, int is_call);
extern int tdx_implied_volatility(double spot, double strike, double rate, double div_yield, double tte, double option_price, int is_call, double* out_iv, double* out_error);
*/
import "C"

import (
	"encoding/json"
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

// callJSON invokes an FFI function that returns a JSON C string,
// parses it, and frees the C memory.
func callJSON(cstr *C.char) (json.RawMessage, error) {
	if cstr == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	goStr := C.GoString(cstr)
	C.tdx_string_free(cstr)
	return json.RawMessage(goStr), nil
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

// DevConfig returns the dev server config (shorter timeouts).
func DevConfig() *Config {
	return &Config{handle: C.tdx_config_dev()}
}

// Close frees the config handle.
func (c *Config) Close() {
	if c.handle != nil {
		C.tdx_config_free(c.handle)
		c.handle = nil
	}
}
