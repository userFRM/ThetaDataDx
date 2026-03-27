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
extern int tdx_fpss_subscribe_open_interest(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_subscribe_full_trades(const TdxFpssHandle* h, const char* sec_type);
extern int tdx_fpss_unsubscribe_quotes(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_unsubscribe_trades(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_unsubscribe_open_interest(const TdxFpssHandle* h, const char* symbol);
extern int tdx_fpss_is_authenticated(const TdxFpssHandle* h);
extern char* tdx_fpss_contract_lookup(const TdxFpssHandle* h, int id);
extern char* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);
extern char* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);
extern void tdx_fpss_shutdown(const TdxFpssHandle* h);
extern void tdx_fpss_free(TdxFpssHandle* h);
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"math"
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

// TradeQuoteTick represents a combined trade + quote tick.
type TradeQuoteTick struct {
	// Trade portion
	MsOfDay        int     `json:"ms_of_day"`
	Sequence       int     `json:"sequence"`
	ExtCondition1  int     `json:"ext_condition1"`
	ExtCondition2  int     `json:"ext_condition2"`
	ExtCondition3  int     `json:"ext_condition3"`
	ExtCondition4  int     `json:"ext_condition4"`
	Condition      int     `json:"condition"`
	Size           int     `json:"size"`
	Exchange       int     `json:"exchange"`
	Price          float64 `json:"price"`
	ConditionFlags int     `json:"condition_flags"`
	PriceFlags     int     `json:"price_flags"`
	VolumeType     int     `json:"volume_type"`
	RecordsBack    int     `json:"records_back"`
	// Quote portion
	QuoteMsOfDay int     `json:"quote_ms_of_day"`
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

// OpenInterestTick represents an open interest data point.
type OpenInterestTick struct {
	MsOfDay      int `json:"ms_of_day"`
	OpenInterest int `json:"open_interest"`
	Date         int `json:"date"`
}

// MarketValueTick represents a market value data point.
type MarketValueTick struct {
	MsOfDay     int     `json:"ms_of_day"`
	MarketCap   float64 `json:"market_cap"`
	SharesOut   int64   `json:"shares_outstanding"`
	EntValue    float64 `json:"enterprise_value"`
	BookValue   float64 `json:"book_value"`
	FreeFloat   int64   `json:"free_float"`
	Date        int     `json:"date"`
}

// GreeksTick represents a Greeks snapshot at a point in time.
type GreeksTick struct {
	MsOfDay   int     `json:"ms_of_day"`
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
	Date      int     `json:"date"`
}

// IVTick represents an implied volatility data point.
type IVTick struct {
	MsOfDay int     `json:"ms_of_day"`
	IV      float64 `json:"iv"`
	IVError float64 `json:"iv_error"`
	Date    int     `json:"date"`
}

// PriceTick represents a price data point (used for index price endpoints).
type PriceTick struct {
	MsOfDay int     `json:"ms_of_day"`
	Price   float64 `json:"price"`
	Date    int     `json:"date"`
}

// CalendarDay represents market calendar information for a single day.
type CalendarDay struct {
	Date      int    `json:"date"`
	IsOpen    bool   `json:"is_open"`
	OpenTime  int    `json:"open_time"`
	CloseTime int    `json:"close_time"`
	Status    string `json:"status"`
}

// InterestRate represents an interest rate data point.
type InterestRate struct {
	Date int     `json:"date"`
	Rate float64 `json:"rate"`
}

// Contract represents an option contract identifier.
type Contract struct {
	Symbol     string `json:"symbol"`
	Expiration string `json:"expiration"`
	Strike     string `json:"strike"`
	Right      string `json:"right"`
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

// ── DataTable parsing ──
//
// The FFI layer returns "raw" endpoints as a columnar DataTable:
//   {"headers": ["col1","col2",...], "rows": [[val1,val2,...], ...]}
//
// Values in rows can be:
//   - numbers (int/float)
//   - strings
//   - price objects: {"value": N, "type": T}
//   - timestamp objects: {"epoch_ms": N, "zone": S}
//   - null
//
// The following helpers parse this format into typed Go structs.

// dataTable is the intermediate representation of a DataTable from the FFI layer.
type dataTable struct {
	Headers []string        `json:"headers"`
	Rows    [][]interface{} `json:"rows"`
}

// parseDataTable unmarshals a JSON DataTable into the intermediate representation.
func parseDataTable(raw json.RawMessage) (*dataTable, error) {
	var dt dataTable
	if err := json.Unmarshal(raw, &dt); err != nil {
		return nil, fmt.Errorf("thetadatadx: parse data table: %w", err)
	}
	return &dt, nil
}

// colIndex returns the index of the named column, or -1 if not found.
func (dt *dataTable) colIndex(name string) int {
	for i, h := range dt.Headers {
		if h == name {
			return i
		}
	}
	return -1
}

// cellNumber extracts a numeric value from a row cell.
// Handles plain numbers and price objects {"value":N,"type":T}.
// Returns 0 for null/missing cells.
func cellNumber(cell interface{}) float64 {
	switch v := cell.(type) {
	case float64:
		return v
	case map[string]interface{}:
		// Price object: {"value": N, "type": T} -- extract value and convert
		if val, ok := v["value"]; ok {
			if n, ok := val.(float64); ok {
				if pt, ok := v["type"]; ok {
					if t, ok := pt.(float64); ok {
						return priceToFloat(n, int(t))
					}
				}
				return n
			}
		}
		return 0
	default:
		return 0
	}
}

// cellInt extracts an integer value from a row cell.
func cellInt(cell interface{}) int {
	switch v := cell.(type) {
	case float64:
		return int(v)
	default:
		return 0
	}
}

// cellInt64 extracts an int64 value from a row cell.
func cellInt64(cell interface{}) int64 {
	switch v := cell.(type) {
	case float64:
		return int64(v)
	default:
		return 0
	}
}

// cellString extracts a string value from a row cell.
func cellString(cell interface{}) string {
	switch v := cell.(type) {
	case string:
		return v
	default:
		return ""
	}
}

// cellBool extracts a boolean value from a row cell.
// Handles both bool literals and numeric 0/1.
func cellBool(cell interface{}) bool {
	switch v := cell.(type) {
	case bool:
		return v
	case float64:
		return v != 0
	default:
		return false
	}
}

// priceToFloat converts a raw price value + type to a float64.
// ThetaData encoding: value * 10^(price_type - 10).
// price_type=0 means "no price" (returns 0.0).
func priceToFloat(value float64, priceType int) float64 {
	if priceType == 0 {
		return 0.0
	}
	exp := priceType - 10
	if exp >= 0 {
		return value * math.Pow(10, float64(exp))
	}
	return value / math.Pow(10, float64(-exp))
}

// safeCell returns the cell at the given column index, or nil if out of range.
func safeCell(row []interface{}, idx int) interface{} {
	if idx < 0 || idx >= len(row) {
		return nil
	}
	return row[idx]
}

// parseTradeQuoteTicks parses a DataTable into TradeQuoteTick structs.
func parseTradeQuoteTicks(raw json.RawMessage) ([]TradeQuoteTick, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	msIdx := dt.colIndex("ms_of_day")
	seqIdx := dt.colIndex("sequence")
	ext1Idx := dt.colIndex("ext_condition1")
	ext2Idx := dt.colIndex("ext_condition2")
	ext3Idx := dt.colIndex("ext_condition3")
	ext4Idx := dt.colIndex("ext_condition4")
	condIdx := dt.colIndex("condition")
	sizeIdx := dt.colIndex("size")
	exgIdx := dt.colIndex("exchange")
	priceIdx := dt.colIndex("price")
	cfIdx := dt.colIndex("condition_flags")
	pfIdx := dt.colIndex("price_flags")
	vtIdx := dt.colIndex("volume_type")
	rbIdx := dt.colIndex("records_back")
	qmsIdx := dt.colIndex("quote_ms_of_day")
	bsIdx := dt.colIndex("bid_size")
	beIdx := dt.colIndex("bid_exchange")
	bidIdx := dt.colIndex("bid")
	bcIdx := dt.colIndex("bid_condition")
	asIdx := dt.colIndex("ask_size")
	aeIdx := dt.colIndex("ask_exchange")
	askIdx := dt.colIndex("ask")
	acIdx := dt.colIndex("ask_condition")
	dateIdx := dt.colIndex("date")

	result := make([]TradeQuoteTick, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, TradeQuoteTick{
			MsOfDay:        cellInt(safeCell(row, msIdx)),
			Sequence:       cellInt(safeCell(row, seqIdx)),
			ExtCondition1:  cellInt(safeCell(row, ext1Idx)),
			ExtCondition2:  cellInt(safeCell(row, ext2Idx)),
			ExtCondition3:  cellInt(safeCell(row, ext3Idx)),
			ExtCondition4:  cellInt(safeCell(row, ext4Idx)),
			Condition:      cellInt(safeCell(row, condIdx)),
			Size:           cellInt(safeCell(row, sizeIdx)),
			Exchange:       cellInt(safeCell(row, exgIdx)),
			Price:          cellNumber(safeCell(row, priceIdx)),
			ConditionFlags: cellInt(safeCell(row, cfIdx)),
			PriceFlags:     cellInt(safeCell(row, pfIdx)),
			VolumeType:     cellInt(safeCell(row, vtIdx)),
			RecordsBack:    cellInt(safeCell(row, rbIdx)),
			QuoteMsOfDay:   cellInt(safeCell(row, qmsIdx)),
			BidSize:        cellInt(safeCell(row, bsIdx)),
			BidExchange:    cellInt(safeCell(row, beIdx)),
			Bid:            cellNumber(safeCell(row, bidIdx)),
			BidCondition:   cellInt(safeCell(row, bcIdx)),
			AskSize:        cellInt(safeCell(row, asIdx)),
			AskExchange:    cellInt(safeCell(row, aeIdx)),
			Ask:            cellNumber(safeCell(row, askIdx)),
			AskCondition:   cellInt(safeCell(row, acIdx)),
			Date:           cellInt(safeCell(row, dateIdx)),
		})
	}
	return result, nil
}

// parseOpenInterestTicks parses a DataTable into OpenInterestTick structs.
func parseOpenInterestTicks(raw json.RawMessage) ([]OpenInterestTick, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	msIdx := dt.colIndex("ms_of_day")
	oiIdx := dt.colIndex("open_interest")
	dateIdx := dt.colIndex("date")

	result := make([]OpenInterestTick, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, OpenInterestTick{
			MsOfDay:      cellInt(safeCell(row, msIdx)),
			OpenInterest: cellInt(safeCell(row, oiIdx)),
			Date:         cellInt(safeCell(row, dateIdx)),
		})
	}
	return result, nil
}

// parseMarketValueTicks parses a DataTable into MarketValueTick structs.
func parseMarketValueTicks(raw json.RawMessage) ([]MarketValueTick, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	msIdx := dt.colIndex("ms_of_day")
	mcIdx := dt.colIndex("market_cap")
	soIdx := dt.colIndex("shares_outstanding")
	evIdx := dt.colIndex("enterprise_value")
	bvIdx := dt.colIndex("book_value")
	ffIdx := dt.colIndex("free_float")
	dateIdx := dt.colIndex("date")

	result := make([]MarketValueTick, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, MarketValueTick{
			MsOfDay:   cellInt(safeCell(row, msIdx)),
			MarketCap: cellNumber(safeCell(row, mcIdx)),
			SharesOut: cellInt64(safeCell(row, soIdx)),
			EntValue:  cellNumber(safeCell(row, evIdx)),
			BookValue: cellNumber(safeCell(row, bvIdx)),
			FreeFloat: cellInt64(safeCell(row, ffIdx)),
			Date:      cellInt(safeCell(row, dateIdx)),
		})
	}
	return result, nil
}

// parseGreeksTicks parses a DataTable into GreeksTick structs.
func parseGreeksTicks(raw json.RawMessage) ([]GreeksTick, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	msIdx := dt.colIndex("ms_of_day")
	valIdx := dt.colIndex("value")
	dIdx := dt.colIndex("delta")
	gIdx := dt.colIndex("gamma")
	tIdx := dt.colIndex("theta")
	vIdx := dt.colIndex("vega")
	rIdx := dt.colIndex("rho")
	ivIdx := dt.colIndex("implied_volatility")
	if ivIdx < 0 { ivIdx = dt.colIndex("iv") }
	iveIdx := dt.colIndex("iv_error")
	vaIdx := dt.colIndex("vanna")
	chIdx := dt.colIndex("charm")
	voIdx := dt.colIndex("vomma")
	veIdx := dt.colIndex("veta")
	spIdx := dt.colIndex("speed")
	zoIdx := dt.colIndex("zomma")
	coIdx := dt.colIndex("color")
	ulIdx := dt.colIndex("ultima")
	d1Idx := dt.colIndex("d1")
	d2Idx := dt.colIndex("d2")
	ddIdx := dt.colIndex("dual_delta")
	dgIdx := dt.colIndex("dual_gamma")
	epIdx := dt.colIndex("epsilon")
	laIdx := dt.colIndex("lambda")
	dateIdx := dt.colIndex("date")

	result := make([]GreeksTick, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, GreeksTick{
			MsOfDay:   cellInt(safeCell(row, msIdx)),
			Value:     cellNumber(safeCell(row, valIdx)),
			Delta:     cellNumber(safeCell(row, dIdx)),
			Gamma:     cellNumber(safeCell(row, gIdx)),
			Theta:     cellNumber(safeCell(row, tIdx)),
			Vega:      cellNumber(safeCell(row, vIdx)),
			Rho:       cellNumber(safeCell(row, rIdx)),
			IV:        cellNumber(safeCell(row, ivIdx)),
			IVError:   cellNumber(safeCell(row, iveIdx)),
			Vanna:     cellNumber(safeCell(row, vaIdx)),
			Charm:     cellNumber(safeCell(row, chIdx)),
			Vomma:     cellNumber(safeCell(row, voIdx)),
			Veta:      cellNumber(safeCell(row, veIdx)),
			Speed:     cellNumber(safeCell(row, spIdx)),
			Zomma:     cellNumber(safeCell(row, zoIdx)),
			Color:     cellNumber(safeCell(row, coIdx)),
			Ultima:    cellNumber(safeCell(row, ulIdx)),
			D1:        cellNumber(safeCell(row, d1Idx)),
			D2:        cellNumber(safeCell(row, d2Idx)),
			DualDelta: cellNumber(safeCell(row, ddIdx)),
			DualGamma: cellNumber(safeCell(row, dgIdx)),
			Epsilon:   cellNumber(safeCell(row, epIdx)),
			Lambda:    cellNumber(safeCell(row, laIdx)),
			Date:      cellInt(safeCell(row, dateIdx)),
		})
	}
	return result, nil
}

// parseIVTicks parses a DataTable into IVTick structs.
func parseIVTicks(raw json.RawMessage) ([]IVTick, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	msIdx := dt.colIndex("ms_of_day")
	ivIdx := dt.colIndex("implied_volatility")
	if ivIdx < 0 { ivIdx = dt.colIndex("iv") }
	iveIdx := dt.colIndex("iv_error")
	dateIdx := dt.colIndex("date")

	result := make([]IVTick, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, IVTick{
			MsOfDay: cellInt(safeCell(row, msIdx)),
			IV:      cellNumber(safeCell(row, ivIdx)),
			IVError: cellNumber(safeCell(row, iveIdx)),
			Date:    cellInt(safeCell(row, dateIdx)),
		})
	}
	return result, nil
}

// parsePriceTicks parses a DataTable into PriceTick structs.
func parsePriceTicks(raw json.RawMessage) ([]PriceTick, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	msIdx := dt.colIndex("ms_of_day")
	priceIdx := dt.colIndex("price")
	dateIdx := dt.colIndex("date")

	result := make([]PriceTick, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, PriceTick{
			MsOfDay: cellInt(safeCell(row, msIdx)),
			Price:   cellNumber(safeCell(row, priceIdx)),
			Date:    cellInt(safeCell(row, dateIdx)),
		})
	}
	return result, nil
}

// parseCalendarDays parses a DataTable into CalendarDay structs.
func parseCalendarDays(raw json.RawMessage) ([]CalendarDay, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	dateIdx := dt.colIndex("date")
	openIdx := dt.colIndex("is_open")
	otIdx := dt.colIndex("open_time")
	ctIdx := dt.colIndex("close_time")
	stIdx := dt.colIndex("status")

	result := make([]CalendarDay, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, CalendarDay{
			Date:      cellInt(safeCell(row, dateIdx)),
			IsOpen:    cellBool(safeCell(row, openIdx)),
			OpenTime:  cellInt(safeCell(row, otIdx)),
			CloseTime: cellInt(safeCell(row, ctIdx)),
			Status:    cellString(safeCell(row, stIdx)),
		})
	}
	return result, nil
}

// parseInterestRates parses a DataTable into InterestRate structs.
func parseInterestRates(raw json.RawMessage) ([]InterestRate, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	dateIdx := dt.colIndex("date")
	rateIdx := dt.colIndex("rate")

	result := make([]InterestRate, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, InterestRate{
			Date: cellInt(safeCell(row, dateIdx)),
			Rate: cellNumber(safeCell(row, rateIdx)),
		})
	}
	return result, nil
}

// parseContracts parses a DataTable into Contract structs.
func parseContracts(raw json.RawMessage) ([]Contract, error) {
	dt, err := parseDataTable(raw)
	if err != nil { return nil, err }

	symIdx := dt.colIndex("symbol")
	if symIdx < 0 { symIdx = dt.colIndex("root") }
	expIdx := dt.colIndex("expiration")
	strIdx := dt.colIndex("strike")
	rIdx := dt.colIndex("right")

	result := make([]Contract, 0, len(dt.Rows))
	for _, row := range dt.Rows {
		result = append(result, Contract{
			Symbol:     cellString(safeCell(row, symIdx)),
			Expiration: cellString(safeCell(row, expIdx)),
			Strike:     fmt.Sprintf("%v", safeCell(row, strIdx)),
			Right:      cellString(safeCell(row, rIdx)),
		})
	}
	return result, nil
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

func (c *Client) StockSnapshotMarketValue(symbols []string) ([]MarketValueTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_stock_snapshot_market_value(c.handle, cJSON))
	if err != nil { return nil, err }
	return parseMarketValueTicks(raw)
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

func (c *Client) StockHistoryTradeQuote(symbol, date string) ([]TradeQuoteTick, error) {
	cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_stock_history_trade_quote(c.handle, cS, cD))
	if err != nil { return nil, err }
	return parseTradeQuoteTicks(raw)
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

func (c *Client) OptionListContracts(requestType, symbol, date string) ([]Contract, error) {
	cRT := C.CString(requestType); cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_option_list_contracts(c.handle, cRT, cS, cD))
	if err != nil { return nil, err }
	return parseContracts(raw)
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

func (c *Client) OptionSnapshotOpenInterest(symbol, expiration, strike, right string) ([]OpenInterestTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_open_interest, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseOpenInterestTicks(raw)
}

func (c *Client) OptionSnapshotMarketValue(symbol, expiration, strike, right string) ([]MarketValueTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_market_value, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseMarketValueTicks(raw)
}

func (c *Client) OptionSnapshotGreeksImpliedVolatility(symbol, expiration, strike, right string) ([]IVTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_greeks_implied_volatility, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseIVTicks(raw)
}

func (c *Client) OptionSnapshotGreeksAll(symbol, expiration, strike, right string) ([]GreeksTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_greeks_all, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionSnapshotGreeksFirstOrder(symbol, expiration, strike, right string) ([]GreeksTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_greeks_first_order, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionSnapshotGreeksSecondOrder(symbol, expiration, strike, right string) ([]GreeksTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_greeks_second_order, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionSnapshotGreeksThirdOrder(symbol, expiration, strike, right string) ([]GreeksTick, error) {
	raw, err := c.optionContractFFI4(C.tdx_option_snapshot_greeks_third_order, symbol, expiration, strike, right)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
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

func (c *Client) OptionHistoryTradeQuote(symbol, expiration, strike, right, date string) ([]TradeQuoteTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_option_history_trade_quote(c.handle, cSym, cExp, cStr, cR, cD))
	if err != nil { return nil, err }
	return parseTradeQuoteTicks(raw)
}

func (c *Client) OptionHistoryOpenInterest(symbol, expiration, strike, right, date string) ([]OpenInterestTick, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_option_history_open_interest(c.handle, cSym, cExp, cStr, cR, cD))
	if err != nil { return nil, err }
	return parseOpenInterestTicks(raw)
}

// ═══════════════════════════════════════════════════════════════════════
//  Option — History Greeks endpoints (11)
// ═══════════════════════════════════════════════════════════════════════

// Helper for raw 6-param endpoints (contract + date + interval)
func (c *Client) optionRaw6(fn func(*C.TdxClient, *C.char, *C.char, *C.char, *C.char, *C.char, *C.char) *C.char, symbol, expiration, strike, right, date, interval string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	return callJSON(fn(c.handle, cSym, cExp, cStr, cR, cD, cI))
}

// Helper for raw 5-param endpoints (contract + date, no interval)
func (c *Client) optionRaw5(fn func(*C.TdxClient, *C.char, *C.char, *C.char, *C.char, *C.char) *C.char, symbol, expiration, strike, right, date string) (json.RawMessage, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR)); defer C.free(unsafe.Pointer(cD))
	return callJSON(fn(c.handle, cSym, cExp, cStr, cR, cD))
}

func (c *Client) OptionHistoryGreeksEOD(symbol, expiration, strike, right, startDate, endDate string) ([]GreeksTick, error) {
	raw, err := c.optionRaw6(C.tdx_option_history_greeks_eod, symbol, expiration, strike, right, startDate, endDate)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryGreeksAll(symbol, expiration, strike, right, date, interval string) ([]GreeksTick, error) {
	raw, err := c.optionRaw6(C.tdx_option_history_greeks_all, symbol, expiration, strike, right, date, interval)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryTradeGreeksAll(symbol, expiration, strike, right, date string) ([]GreeksTick, error) {
	raw, err := c.optionRaw5(C.tdx_option_history_trade_greeks_all, symbol, expiration, strike, right, date)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryGreeksFirstOrder(symbol, expiration, strike, right, date, interval string) ([]GreeksTick, error) {
	raw, err := c.optionRaw6(C.tdx_option_history_greeks_first_order, symbol, expiration, strike, right, date, interval)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryTradeGreeksFirstOrder(symbol, expiration, strike, right, date string) ([]GreeksTick, error) {
	raw, err := c.optionRaw5(C.tdx_option_history_trade_greeks_first_order, symbol, expiration, strike, right, date)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryGreeksSecondOrder(symbol, expiration, strike, right, date, interval string) ([]GreeksTick, error) {
	raw, err := c.optionRaw6(C.tdx_option_history_greeks_second_order, symbol, expiration, strike, right, date, interval)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryTradeGreeksSecondOrder(symbol, expiration, strike, right, date string) ([]GreeksTick, error) {
	raw, err := c.optionRaw5(C.tdx_option_history_trade_greeks_second_order, symbol, expiration, strike, right, date)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryGreeksThirdOrder(symbol, expiration, strike, right, date, interval string) ([]GreeksTick, error) {
	raw, err := c.optionRaw6(C.tdx_option_history_greeks_third_order, symbol, expiration, strike, right, date, interval)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryTradeGreeksThirdOrder(symbol, expiration, strike, right, date string) ([]GreeksTick, error) {
	raw, err := c.optionRaw5(C.tdx_option_history_trade_greeks_third_order, symbol, expiration, strike, right, date)
	if err != nil { return nil, err }
	return parseGreeksTicks(raw)
}

func (c *Client) OptionHistoryGreeksImpliedVolatility(symbol, expiration, strike, right, date, interval string) ([]IVTick, error) {
	raw, err := c.optionRaw6(C.tdx_option_history_greeks_implied_volatility, symbol, expiration, strike, right, date, interval)
	if err != nil { return nil, err }
	return parseIVTicks(raw)
}

func (c *Client) OptionHistoryTradeGreeksImpliedVolatility(symbol, expiration, strike, right, date string) ([]IVTick, error) {
	raw, err := c.optionRaw5(C.tdx_option_history_trade_greeks_implied_volatility, symbol, expiration, strike, right, date)
	if err != nil { return nil, err }
	return parseIVTicks(raw)
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

func (c *Client) IndexSnapshotPrice(symbols []string) ([]PriceTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_index_snapshot_price(c.handle, cJSON))
	if err != nil { return nil, err }
	return parsePriceTicks(raw)
}

func (c *Client) IndexSnapshotMarketValue(symbols []string) ([]MarketValueTick, error) {
	cJSON, err := symbolsToJSON(symbols); if err != nil { return nil, err }
	defer C.free(unsafe.Pointer(cJSON))
	raw, err := callJSON(C.tdx_index_snapshot_market_value(c.handle, cJSON))
	if err != nil { return nil, err }
	return parseMarketValueTicks(raw)
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

func (c *Client) IndexHistoryPrice(symbol, date, interval string) ([]PriceTick, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	raw, err := callJSON(C.tdx_index_history_price(c.handle, cS, cD, cI))
	if err != nil { return nil, err }
	return parsePriceTicks(raw)
}

// ═══════════════════════════════════════════════════════════════════════
//  Index — At-Time endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) IndexAtTimePrice(symbol, startDate, endDate, timeOfDay string) ([]PriceTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	raw, err := callJSON(C.tdx_index_at_time_price(c.handle, cS, cSt, cEn, cT))
	if err != nil { return nil, err }
	return parsePriceTicks(raw)
}

// ═══════════════════════════════════════════════════════════════════════
//  Calendar endpoints (3)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) CalendarOpenToday() ([]CalendarDay, error) {
	raw, err := callJSON(C.tdx_calendar_open_today(c.handle))
	if err != nil { return nil, err }
	return parseCalendarDays(raw)
}

func (c *Client) CalendarOnDate(date string) ([]CalendarDay, error) {
	cD := C.CString(date); defer C.free(unsafe.Pointer(cD))
	raw, err := callJSON(C.tdx_calendar_on_date(c.handle, cD))
	if err != nil { return nil, err }
	return parseCalendarDays(raw)
}

func (c *Client) CalendarYear(year string) ([]CalendarDay, error) {
	cY := C.CString(year); defer C.free(unsafe.Pointer(cY))
	raw, err := callJSON(C.tdx_calendar_year(c.handle, cY))
	if err != nil { return nil, err }
	return parseCalendarDays(raw)
}

// ═══════════════════════════════════════════════════════════════════════
//  Interest Rate endpoints (1)
// ═══════════════════════════════════════════════════════════════════════

func (c *Client) InterestRateHistoryEOD(symbol, startDate, endDate string) ([]InterestRate, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	raw, err := callJSON(C.tdx_interest_rate_history_eod(c.handle, cS, cSt, cEn))
	if err != nil { return nil, err }
	return parseInterestRates(raw)
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

// SubscribeOpenInterest subscribes to open interest data for a stock symbol.
//
// Returns the request ID for this subscription.
func (f *FpssClient) SubscribeOpenInterest(symbol string) (int, error) {
	cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cSym))
	rc := C.tdx_fpss_subscribe_open_interest(f.handle, cSym)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// SubscribeFullTrades subscribes to all trades for a security type.
//
// secType must be one of: "STOCK", "OPTION", "INDEX".
// Returns the request ID for this subscription.
func (f *FpssClient) SubscribeFullTrades(secType string) (int, error) {
	cType := C.CString(secType)
	defer C.free(unsafe.Pointer(cType))
	rc := C.tdx_fpss_subscribe_full_trades(f.handle, cType)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// UnsubscribeQuotes unsubscribes from quote data for a stock symbol.
//
// Returns the request ID.
func (f *FpssClient) UnsubscribeQuotes(symbol string) (int, error) {
	cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cSym))
	rc := C.tdx_fpss_unsubscribe_quotes(f.handle, cSym)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// UnsubscribeTrades unsubscribes from trade data for a stock symbol.
//
// Returns the request ID.
func (f *FpssClient) UnsubscribeTrades(symbol string) (int, error) {
	cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cSym))
	rc := C.tdx_fpss_unsubscribe_trades(f.handle, cSym)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// UnsubscribeOpenInterest unsubscribes from open interest data for a stock symbol.
//
// Returns the request ID.
func (f *FpssClient) UnsubscribeOpenInterest(symbol string) (int, error) {
	cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cSym))
	rc := C.tdx_fpss_unsubscribe_open_interest(f.handle, cSym)
	if rc < 0 {
		return 0, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// IsAuthenticated returns true if the FPSS client is currently authenticated.
func (f *FpssClient) IsAuthenticated() bool {
	return C.tdx_fpss_is_authenticated(f.handle) != 0
}

// ContractLookup looks up a single contract by its server-assigned ID.
//
// Returns the contract string, or an error if not found.
func (f *FpssClient) ContractLookup(id int) (string, error) {
	cstr := C.tdx_fpss_contract_lookup(f.handle, C.int(id))
	if cstr == nil {
		return "", fmt.Errorf("thetadatadx: contract %d not found", id)
	}
	goStr := C.GoString(cstr)
	C.tdx_string_free(cstr)
	return goStr, nil
}

// ActiveSubscriptions returns a JSON array of currently active subscriptions.
func (f *FpssClient) ActiveSubscriptions() (json.RawMessage, error) {
	cstr := C.tdx_fpss_active_subscriptions(f.handle)
	if cstr == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	goStr := C.GoString(cstr)
	C.tdx_string_free(cstr)
	return json.RawMessage(goStr), nil
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
