package thetadatadx

/*
#include <stdlib.h>
#include <stdint.h>

// Forward declarations (already defined in thetadatadx.go, but CGo needs them per file).
typedef void TdxCredentials;
typedef void TdxClient;
typedef void TdxConfig;
typedef void TdxFpssHandle;

extern TdxClient* tdx_client_connect(const TdxCredentials* creds, const TdxConfig* config);
extern void tdx_client_free(TdxClient* client);

// Stock — List
extern char* tdx_stock_list_symbols(const TdxClient* client);
extern char* tdx_stock_list_dates(const TdxClient* client, const char* request_type, const char* symbol);

// Stock — Snapshot
extern char* tdx_stock_snapshot_ohlc(const TdxClient* client, const char* symbols_json);
extern char* tdx_stock_snapshot_trade(const TdxClient* client, const char* symbols_json);
extern char* tdx_stock_snapshot_quote(const TdxClient* client, const char* symbols_json);
extern char* tdx_stock_snapshot_market_value(const TdxClient* client, const char* symbols_json);

// Stock — History
extern char* tdx_stock_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern char* tdx_stock_history_ohlc(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern char* tdx_stock_history_ohlc_range(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern char* tdx_stock_history_trade(const TdxClient* client, const char* symbol, const char* date);
extern char* tdx_stock_history_quote(const TdxClient* client, const char* symbol, const char* date, const char* interval);
extern char* tdx_stock_history_trade_quote(const TdxClient* client, const char* symbol, const char* date);

// Stock — At-Time
extern char* tdx_stock_at_time_trade(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);
extern char* tdx_stock_at_time_quote(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);

// Option — List
extern char* tdx_option_list_symbols(const TdxClient* client);
extern char* tdx_option_list_dates(const TdxClient* client, const char* request_type, const char* symbol, const char* expiration, const char* strike, const char* right);
extern char* tdx_option_list_expirations(const TdxClient* client, const char* symbol);
extern char* tdx_option_list_strikes(const TdxClient* client, const char* symbol, const char* expiration);
extern char* tdx_option_list_contracts(const TdxClient* client, const char* request_type, const char* symbol, const char* date);

// Option — Snapshot
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

// Option — History
extern char* tdx_option_history_eod(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date);
extern char* tdx_option_history_ohlc(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date, const char* interval);
extern char* tdx_option_history_trade_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);
extern char* tdx_option_history_open_interest(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* date);

// Option — History Greeks
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

// Option — At-Time
extern char* tdx_option_at_time_trade(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);
extern char* tdx_option_at_time_quote(const TdxClient* client, const char* symbol, const char* expiration, const char* strike, const char* right, const char* start_date, const char* end_date, const char* time_of_day);

// Index — List
extern char* tdx_index_list_symbols(const TdxClient* client);
extern char* tdx_index_list_dates(const TdxClient* client, const char* symbol);

// Index — Snapshot
extern char* tdx_index_snapshot_ohlc(const TdxClient* client, const char* symbols_json);
extern char* tdx_index_snapshot_price(const TdxClient* client, const char* symbols_json);
extern char* tdx_index_snapshot_market_value(const TdxClient* client, const char* symbols_json);

// Index — History
extern char* tdx_index_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);
extern char* tdx_index_history_ohlc(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* interval);
extern char* tdx_index_history_price(const TdxClient* client, const char* symbol, const char* date, const char* interval);

// Index — At-Time
extern char* tdx_index_at_time_price(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date, const char* time_of_day);

// Calendar
extern char* tdx_calendar_open_today(const TdxClient* client);
extern char* tdx_calendar_on_date(const TdxClient* client, const char* date);
extern char* tdx_calendar_year(const TdxClient* client, const char* year);

// Interest Rate
extern char* tdx_interest_rate_history_eod(const TdxClient* client, const char* symbol, const char* start_date, const char* end_date);

// Greeks
extern char* tdx_all_greeks(double spot, double strike, double rate, double div_yield, double tte, double option_price, int is_call);
extern int tdx_implied_volatility(double spot, double strike, double rate, double div_yield, double tte, double option_price, int is_call, double* out_iv, double* out_error);
extern void tdx_string_free(char* s);

// FPSS (real-time streaming)
extern TdxFpssHandle* tdx_fpss_connect(const TdxCredentials* creds, const TdxConfig* config);
extern int tdx_fpss_subscribe_quotes(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_subscribe_trades(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_unsubscribe_quotes(const TdxFpssHandle* h, const char* symbol);
extern char* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);
extern void tdx_fpss_shutdown(const TdxFpssHandle* h);
extern void tdx_fpss_free(TdxFpssHandle* h);
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

// ── Tick types ──

// EodTick represents an end-of-day tick.
type EodTick struct {
	MsOfDay int     `json:"ms_of_day"`
	Open    float64 `json:"open"`
	High    float64 `json:"high"`
	Low     float64 `json:"low"`
	Close   float64 `json:"close"`
	Volume  int     `json:"volume"`
	Count   int     `json:"count"`
	Bid     float64 `json:"bid"`
	Ask     float64 `json:"ask"`
	Date    int     `json:"date"`
}

// OhlcTick represents an OHLC bar.
type OhlcTick struct {
	MsOfDay int     `json:"ms_of_day"`
	Open    float64 `json:"open"`
	High    float64 `json:"high"`
	Low     float64 `json:"low"`
	Close   float64 `json:"close"`
	Volume  int     `json:"volume"`
	Count   int     `json:"count"`
	Date    int     `json:"date"`
}

// TradeTick represents a trade.
type TradeTick struct {
	MsOfDay        int     `json:"ms_of_day"`
	Sequence       int     `json:"sequence"`
	Condition      int     `json:"condition"`
	Size           int     `json:"size"`
	Exchange       int     `json:"exchange"`
	Price          float64 `json:"price"`
	PriceRaw       int     `json:"price_raw"`
	PriceType      int     `json:"price_type"`
	ConditionFlags int     `json:"condition_flags"`
	PriceFlags     int     `json:"price_flags"`
	VolumeType     int     `json:"volume_type"`
	RecordsBack    int     `json:"records_back"`
	Date           int     `json:"date"`
}

// QuoteTick represents an NBBO quote.
type QuoteTick struct {
	MsOfDay      int     `json:"ms_of_day"`
	BidSize      int     `json:"bid_size"`
	BidExchange  int     `json:"bid_exchange"`
	Bid          float64 `json:"bid"`
	BidCondition int     `json:"bid_condition"`
	AskSize      int     `json:"ask_size"`
	AskExchange  int     `json:"ask_exchange"`
	Ask          float64 `json:"ask"`
	AskCondition int     `json:"ask_condition"`
	Date         int     `json:"date"`
}

// Greeks holds all 22 Black-Scholes Greeks plus IV.
type Greeks struct {
	Value     float64 `json:"value"`
	Delta     float64 `json:"delta"`
	Gamma     float64 `json:"gamma"`
	Theta     float64 `json:"theta"`
	Vega      float64 `json:"vega"`
	Rho       float64 `json:"rho"`
	IV        float64 `json:"iv"`
	IVError   float64 `json:"iv_error"`
	Vanna     float64 `json:"vanna"`
	Charm     float64 `json:"charm"`
	Vomma     float64 `json:"vomma"`
	Veta      float64 `json:"veta"`
	Speed     float64 `json:"speed"`
	Zomma     float64 `json:"zomma"`
	Color     float64 `json:"color"`
	Ultima    float64 `json:"ultima"`
	D1        float64 `json:"d1"`
	D2        float64 `json:"d2"`
	DualDelta float64 `json:"dual_delta"`
	DualGamma float64 `json:"dual_gamma"`
	Epsilon   float64 `json:"epsilon"`
	Lambda    float64 `json:"lambda"`
}

// ── Internal helpers ──

func parseStrings(raw json.RawMessage) ([]string, error) {
	var result []string
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, fmt.Errorf("thetadatadx: parse strings: %w", err)
	}
	return result, nil
}

func symbolsToJSON(symbols []string) (*C.char, error) {
	b, err := json.Marshal(symbols)
	if err != nil {
		return nil, fmt.Errorf("thetadatadx: serialize symbols: %w", err)
	}
	return C.CString(string(b)), nil
}

// ── Client ──

// Client is a high-level ThetaData client.
type Client struct {
	handle *C.TdxClient
}

// Connect authenticates and connects to ThetaData servers.
func Connect(creds *Credentials, config *Config) (*Client, error) {
	if creds == nil || creds.handle == nil {
		return nil, fmt.Errorf("thetadatadx: credentials handle is nil")
	}
	if config == nil || config.handle == nil {
		return nil, fmt.Errorf("thetadatadx: config handle is nil")
	}
	h := C.tdx_client_connect(creds.handle, config.handle)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &Client{handle: h}, nil
}

// Close disconnects and frees the client handle.
func (c *Client) Close() {
	if c.handle != nil {
		C.tdx_client_free(c.handle)
		c.handle = nil
	}
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) StockListSymbols() ([]string, error) {
	raw, err := callJSON(C.tdx_stock_list_symbols(c.handle))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

func (c *Client) StockListDates(requestType, symbol string) ([]string, error) {
	cRT := C.CString(requestType); cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cSym))
	raw, err := callJSON(C.tdx_stock_list_dates(c.handle, cRT, cSym))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — Snapshot endpoints (4)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) StockSnapshotOHLC(symbols []string) ([]OhlcTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_stock_snapshot_ohlc(c.handle, cJSON))
	if err != nil { return nil, err }
	var result []OhlcTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockSnapshotTrade(symbols []string) ([]TradeTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_stock_snapshot_trade(c.handle, cJSON))
	if err != nil { return nil, err }
	var result []TradeTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockSnapshotQuote(symbols []string) ([]QuoteTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_stock_snapshot_quote(c.handle, cJSON))
	if err != nil { return nil, err }
	var result []QuoteTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockSnapshotMarketValue(symbols []string) (json.RawMessage, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	return callJSON(C.tdx_stock_snapshot_market_value(c.handle, cJSON))
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — History endpoints (5 + bonus)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) StockHistoryEOD(symbol, startDate, endDate string) ([]EodTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	raw, err := callJSON(C.tdx_stock_history_eod(c.handle, cS, cSt, cEn))
	if err != nil { return nil, err }
	var result []EodTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockHistoryOHLC(symbol, date, interval string) ([]OhlcTick, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_stock_history_ohlc(c.handle, cS, cD, cI))
	if err != nil { return nil, err }
	var result []OhlcTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockHistoryOHLCRange(symbol, startDate, endDate, interval string) ([]OhlcTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_stock_history_ohlc_range(c.handle, cS, cSt, cEn, cI))
	if err != nil { return nil, err }
	var result []OhlcTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockHistoryTrade(symbol, date string) ([]TradeTick, error) {
	cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_stock_history_trade(c.handle, cS, cD))
	if err != nil { return nil, err }
	var result []TradeTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockHistoryQuote(symbol, date, interval string) ([]QuoteTick, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_stock_history_quote(c.handle, cS, cD, cI))
	if err != nil { return nil, err }
	var result []QuoteTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockHistoryTradeQuote(symbol, date string) (json.RawMessage, error) {
	cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	return callJSON(C.tdx_stock_history_trade_quote(c.handle, cS, cD))
}

// ═══════════════════════════════════════════════════════════════════════
//  Stock — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) StockAtTimeTrade(symbol, startDate, endDate, timeOfDay string) ([]TradeTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	raw, err := callJSON(C.tdx_stock_at_time_trade(c.handle, cS, cSt, cEn, cT))
	if err != nil { return nil, err }
	var result []TradeTick
	return result, json.Unmarshal(raw, &result)
}

func (c *Client) StockAtTimeQuote(symbol, startDate, endDate, timeOfDay string) ([]QuoteTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	raw, err := callJSON(C.tdx_stock_at_time_quote(c.handle, cS, cSt, cEn, cT))
	if err != nil { return nil, err }
	var result []QuoteTick
	return result, json.Unmarshal(raw, &result)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — List endpoints (5)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) OptionListSymbols() ([]string, error) {
	raw, err := callJSON(C.tdx_option_list_symbols(c.handle))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

func (c *Client) OptionListDates(requestType, symbol, expiration, strike, right string) ([]string, error) {
	cRT := C.CString(requestType); cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR))
	raw, err := callJSON(C.tdx_option_list_dates(c.handle, cRT, cSym, cExp, cStr, cR))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

func (c *Client) OptionListExpirations(symbol string) ([]string, error) {
	cS := C.CString(symbol); defer C.free(unsafe.Pointer(cS))
	raw, err := callJSON(C.tdx_option_list_expirations(c.handle, cS))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

func (c *Client) OptionListStrikes(symbol, expiration string) ([]string, error) {
	cS := C.CString(symbol); cE := C.CString(expiration)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cE))
	raw, err := callJSON(C.tdx_option_list_strikes(c.handle, cS, cE))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

func (c *Client) OptionListContracts(requestType, symbol, date string) (json.RawMessage, error) {
	cRT := C.CString(requestType); cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	return callJSON(C.tdx_option_list_contracts(c.handle, cRT, cS, cD))
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — Snapshot endpoints (10)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) optionContractFFI4(fn func(*C.TdxClient, *C.char, *C.char, *C.char, *C.char) *C.char, symbol, expiration, strike, right string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR))
	return callJSON(fn(c.handle, cSym, cExp, cStr, cR))
}

func (c *Client) OptionSnapshotOHLC(symbol, expiration, strike, right string) ([]OhlcTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_ohlc, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	var result []OhlcTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionSnapshotTrade(symbol, expiration, strike, right string) ([]TradeTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_trade, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	var result []TradeTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionSnapshotQuote(symbol, expiration, strike, right string) ([]QuoteTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_quote, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	var result []QuoteTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionSnapshotOpenInterest(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_open_interest, symbol, expiration, strike, right)
}

func (c *Client) OptionSnapshotMarketValue(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_market_value, symbol, expiration, strike, right)
}

func (c *Client) OptionSnapshotGreeksImpliedVolatility(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_greeks_implied_volatility, symbol, expiration, strike, right)
}

func (c *Client) OptionSnapshotGreeksAll(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_greeks_all, symbol, expiration, strike, right)
}

func (c *Client) OptionSnapshotGreeksFirstOrder(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_greeks_first_order, symbol, expiration, strike, right)
}

func (c *Client) OptionSnapshotGreeksSecondOrder(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_greeks_second_order, symbol, expiration, strike, right)
}

func (c *Client) OptionSnapshotGreeksThirdOrder(symbol, expiration, strike, right string) (json.RawMessage, error) {
	return c.optionContractFFI4(C.tdx_option_snapshot_greeks_third_order, symbol, expiration, strike, right)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History endpoints (6)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) OptionHistoryEOD(symbol, expiration, strike, right, startDate, endDate string) ([]EodTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	raw, err := callJSON(C.tdx_option_history_eod(c.handle, cSym, cExp, cStr, cR, cSt, cEn))
	if err != nil { return nil, err }
	var result []EodTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionHistoryOHLC(symbol, expiration, strike, right, date, interval string) ([]OhlcTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_option_history_ohlc(c.handle, cSym, cExp, cStr, cR, cD, cI))
	if err != nil { return nil, err }
	var result []OhlcTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionHistoryTrade(symbol, expiration, strike, right, date string) ([]TradeTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_option_history_trade(c.handle, cSym, cExp, cStr, cR, cD))
	if err != nil { return nil, err }
	var result []TradeTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionHistoryQuote(symbol, expiration, strike, right, date, interval string) ([]QuoteTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_option_history_quote(c.handle, cSym, cExp, cStr, cR, cD, cI))
	if err != nil { return nil, err }
	var result []QuoteTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionHistoryTradeQuote(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	return callJSON(C.tdx_option_history_trade_quote(c.handle, cSym, cExp, cStr, cR, cD))
}

func (c *Client) OptionHistoryOpenInterest(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	return callJSON(C.tdx_option_history_open_interest(c.handle, cSym, cExp, cStr, cR, cD))
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints (11)
// ═══════════════════════════════════════════════════════════════════════

// Helper for Greeks 6-param endpoints (contract + date + interval)
func (c *Client) optionGreeks6(fn func(*C.TdxClient, *C.char, *C.char, *C.char, *C.char, *C.char, *C.char) *C.char, symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	return callJSON(fn(c.handle, cSym, cExp, cStr, cR, cD, cI))
}

// Helper for Greeks 5-param endpoints (contract + date, no interval)
func (c *Client) optionGreeks5(fn func(*C.TdxClient, *C.char, *C.char, *C.char, *C.char, *C.char) *C.char, symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	return callJSON(fn(c.handle, cSym, cExp, cStr, cR, cD))
}

func (c *Client) OptionHistoryGreeksEOD(symbol, expiration, strike, right, startDate, endDate string) (json.RawMessage, error) {
	return c.optionGreeks6(C.tdx_option_history_greeks_eod, symbol, expiration, strike, right, startDate, endDate)
}

func (c *Client) OptionHistoryGreeksAll(symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	return c.optionGreeks6(C.tdx_option_history_greeks_all, symbol, expiration, strike, right, date, interval)
}

func (c *Client) OptionHistoryTradeGreeksAll(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	return c.optionGreeks5(C.tdx_option_history_trade_greeks_all, symbol, expiration, strike, right, date)
}

func (c *Client) OptionHistoryGreeksFirstOrder(symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	return c.optionGreeks6(C.tdx_option_history_greeks_first_order, symbol, expiration, strike, right, date, interval)
}

func (c *Client) OptionHistoryTradeGreeksFirstOrder(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	return c.optionGreeks5(C.tdx_option_history_trade_greeks_first_order, symbol, expiration, strike, right, date)
}

func (c *Client) OptionHistoryGreeksSecondOrder(symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	return c.optionGreeks6(C.tdx_option_history_greeks_second_order, symbol, expiration, strike, right, date, interval)
}

func (c *Client) OptionHistoryTradeGreeksSecondOrder(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	return c.optionGreeks5(C.tdx_option_history_trade_greeks_second_order, symbol, expiration, strike, right, date)
}

func (c *Client) OptionHistoryGreeksThirdOrder(symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	return c.optionGreeks6(C.tdx_option_history_greeks_third_order, symbol, expiration, strike, right, date, interval)
}

func (c *Client) OptionHistoryTradeGreeksThirdOrder(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	return c.optionGreeks5(C.tdx_option_history_trade_greeks_third_order, symbol, expiration, strike, right, date)
}

func (c *Client) OptionHistoryGreeksImpliedVolatility(symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	return c.optionGreeks6(C.tdx_option_history_greeks_implied_volatility, symbol, expiration, strike, right, date, interval)
}

func (c *Client) OptionHistoryTradeGreeksImpliedVolatility(symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	return c.optionGreeks5(C.tdx_option_history_trade_greeks_implied_volatility, symbol, expiration, strike, right, date)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — At-Time endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) OptionAtTimeTrade(symbol, expiration, strike, right, startDate, endDate, timeOfDay string) ([]TradeTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right)
	cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR))
	defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	raw, err := callJSON(C.tdx_option_at_time_trade(c.handle, cSym, cExp, cStr, cR, cSt, cEn, cT))
	if err != nil { return nil, err }
	var result []TradeTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) OptionAtTimeQuote(symbol, expiration, strike, right, startDate, endDate, timeOfDay string) ([]QuoteTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right)
	cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR))
	defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	raw, err := callJSON(C.tdx_option_at_time_quote(c.handle, cSym, cExp, cStr, cR, cSt, cEn, cT))
	if err != nil { return nil, err }
	var result []QuoteTick; return result, json.Unmarshal(raw, &result)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — List endpoints (2)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) IndexListSymbols() ([]string, error) {
	raw, err := callJSON(C.tdx_index_list_symbols(c.handle))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

func (c *Client) IndexListDates(symbol string) ([]string, error) {
	cS := C.CString(symbol); defer C.free(unsafe.Pointer(cS))
	raw, err := callJSON(C.tdx_index_list_dates(c.handle, cS))
	if err != nil { return nil, err }
	return parseStrings(raw)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — Snapshot endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) IndexSnapshotOHLC(symbols []string) ([]OhlcTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_index_snapshot_ohlc(c.handle, cJSON))
	if err != nil { return nil, err }
	var result []OhlcTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) IndexSnapshotPrice(symbols []string) (json.RawMessage, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	return callJSON(C.tdx_index_snapshot_price(c.handle, cJSON))
}

func (c *Client) IndexSnapshotMarketValue(symbols []string) (json.RawMessage, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	return callJSON(C.tdx_index_snapshot_market_value(c.handle, cJSON))
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — History endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) IndexHistoryEOD(symbol, startDate, endDate string) ([]EodTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	raw, err := callJSON(C.tdx_index_history_eod(c.handle, cS, cSt, cEn))
	if err != nil { return nil, err }
	var result []EodTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) IndexHistoryOHLC(symbol, startDate, endDate, interval string) ([]OhlcTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_index_history_ohlc(c.handle, cS, cSt, cEn, cI))
	if err != nil { return nil, err }
	var result []OhlcTick; return result, json.Unmarshal(raw, &result)
}

func (c *Client) IndexHistoryPrice(symbol, date, interval string) (json.RawMessage, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	return callJSON(C.tdx_index_history_price(c.handle, cS, cD, cI))
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — At-Time endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) IndexAtTimePrice(symbol, startDate, endDate, timeOfDay string) (json.RawMessage, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	return callJSON(C.tdx_index_at_time_price(c.handle, cS, cSt, cEn, cT))
}

// ═══════════════════════════════════════════════════════════════════════
//  Calendar endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) CalendarOpenToday() (json.RawMessage, error) {
	return callJSON(C.tdx_calendar_open_today(c.handle))
}

func (c *Client) CalendarOnDate(date string) (json.RawMessage, error) {
	cD := C.CString(date); defer C.free(unsafe.Pointer(cD))
	return callJSON(C.tdx_calendar_on_date(c.handle, cD))
}

func (c *Client) CalendarYear(year string) (json.RawMessage, error) {
	cY := C.CString(year); defer C.free(unsafe.Pointer(cY))
	return callJSON(C.tdx_calendar_year(c.handle, cY))
}

// ═══════════════════════════════════════════════════════════════════════
//  Interest Rate endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) InterestRateHistoryEOD(symbol, startDate, endDate string) (json.RawMessage, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	return callJSON(C.tdx_interest_rate_history_eod(c.handle, cS, cSt, cEn))
}

// ═══════════════════════════════════════════════════════════════════════
//  Greeks (standalone)
// ═══════════════════════════════════════════════════════════════════════

// AllGreeks computes all 22 Black-Scholes Greeks + IV in one call.
func AllGreeks(spot, strike, rate, divYield, tte, optionPrice float64, isCall bool) (*Greeks, error) {
	callFlag := C.int(0)
	if isCall {
		callFlag = 1
	}
	cstr := C.tdx_all_greeks(
		C.double(spot), C.double(strike), C.double(rate),
		C.double(divYield), C.double(tte), C.double(optionPrice),
		callFlag,
	)
	raw, err := callJSON(cstr)
	if err != nil {
		return nil, err
	}
	var g Greeks
	if err := json.Unmarshal(raw, &g); err != nil {
		return nil, fmt.Errorf("thetadatadx: failed to parse greeks: %w", err)
	}
	return &g, nil
}

// ImpliedVolatility computes implied volatility via bisection.
// Returns (iv, error_bound).
func ImpliedVolatility(spot, strike, rate, divYield, tte, optionPrice float64, isCall bool) (float64, float64, error) {
	callFlag := C.int(0)
	if isCall {
		callFlag = 1
	}
	var outIV, outErr C.double
	rc := C.tdx_implied_volatility(
		C.double(spot), C.double(strike), C.double(rate),
		C.double(divYield), C.double(tte), C.double(optionPrice),
		callFlag, &outIV, &outErr,
	)
	if rc != 0 {
		return 0, 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return float64(outIV), float64(outErr), nil
}

// ═══════════════════════════════════════════════════════════════════════
//  FPSS — Real-time streaming client
// ═══════════════════════════════════════════════════════════════════════

// FpssClient is a real-time FPSS streaming client.
//
// Events are buffered internally. Call NextEvent() to poll for events.
type FpssClient struct {
	handle *C.TdxFpssHandle
}

// NewFpssClient connects to FPSS streaming servers.
//
// Events are collected in an internal queue. Call NextEvent() to poll.
func NewFpssClient(creds *Credentials, config *Config) (*FpssClient, error) {
	if creds == nil || creds.handle == nil {
		return nil, fmt.Errorf("thetadatadx: credentials handle is nil")
	}
	if config == nil || config.handle == nil {
		return nil, fmt.Errorf("thetadatadx: config handle is nil")
	}
	h := C.tdx_fpss_connect(creds.handle, config.handle)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &FpssClient{handle: h}, nil
}

// SubscribeQuotes subscribes to quote data for a stock symbol.
//
// Returns the request ID for this subscription.
func (f *FpssClient) SubscribeQuotes(symbol string) (int, error) {
	cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cSym))
	rc := C.tdx_fpss_subscribe_quotes(f.handle, cSym)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// SubscribeTrades subscribes to trade data for a stock symbol.
//
// Returns the request ID for this subscription.
func (f *FpssClient) SubscribeTrades(symbol string) (int, error) {
	cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cSym))
	rc := C.tdx_fpss_subscribe_trades(f.handle, cSym)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// NextEvent polls for the next FPSS event with a timeout.
//
// Returns nil (not an error) if no event arrived within the timeout.
func (f *FpssClient) NextEvent(timeoutMs uint64) (json.RawMessage, error) {
	cstr := C.tdx_fpss_next_event(f.handle, C.uint64_t(timeoutMs))
	if cstr == nil {
		// Timeout — not an error.
		return nil, nil
	}
	goStr := C.GoString(cstr)
	C.tdx_string_free(cstr)
	return json.RawMessage(goStr), nil
}

// Shutdown stops the FPSS client and all background threads.
func (f *FpssClient) Shutdown() {
	if f.handle != nil {
		C.tdx_fpss_shutdown(f.handle)
	}
}

// Close frees the FPSS handle. Must be called after Shutdown().
func (f *FpssClient) Close() {
	if f.handle != nil {
		C.tdx_fpss_free(f.handle)
		f.handle = nil
	}
}
