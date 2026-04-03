package thetadatadx

/*
#include <stdlib.h>
#include <stdint.h>
#include <stddef.h>

// Forward declarations (already defined in thetadx.go, but CGo needs them per file).
typedef void TdxCredentials;
typedef void TdxConfig;
typedef void TdxFpssHandle;

// FPSS event kind tag.
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
    double bid_f64;
    int32_t bid_condition;
    int32_t ask_size;
    int32_t ask_exchange;
    int32_t ask;
    double ask_f64;
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
    double price_f64;
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
    double open_f64;
    int32_t high;
    double high_f64;
    int32_t low;
    double low_f64;
    int32_t close;
    double close_f64;
    int64_t volume;
    int64_t count;
    int32_t price_type;
    int32_t date;
    uint64_t received_at_ns;
} TdxFpssOhlcvc;

typedef struct {
    int32_t kind;
    int32_t id;
    const char* detail;
} TdxFpssControl;

typedef struct {
    uint8_t code;
    const uint8_t* payload;
    size_t payload_len;
} TdxFpssRawData;

typedef struct {
    TdxFpssEventKind kind;
    TdxFpssQuote quote;
    TdxFpssTrade trade;
    TdxFpssOpenInterest open_interest;
    TdxFpssOhlcvc ohlcvc;
    TdxFpssControl control;
    TdxFpssRawData raw_data;
} TdxFpssEvent;

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
// ── Subscription types ──
typedef struct {
    const char* kind;
    const char* contract;
} TdxSubscription;

typedef struct {
    const TdxSubscription* data;
    size_t len;
} TdxSubscriptionArray;

extern TdxSubscriptionArray* tdx_fpss_active_subscriptions(const TdxFpssHandle* h);
extern void tdx_subscription_array_free(TdxSubscriptionArray* arr);
extern TdxFpssEvent* tdx_fpss_next_event(const TdxFpssHandle* h, uint64_t timeout_ms);
extern void tdx_fpss_event_free(TdxFpssEvent* event);
extern void tdx_fpss_shutdown(const TdxFpssHandle* h);
extern void tdx_fpss_free(TdxFpssHandle* h);
extern const char* tdx_last_error();
extern void tdx_string_free(char* s);
*/
import "C"

import (
	"fmt"
	"math"
	"unsafe"
)

// PriceToF64 converts a ThetaData price-encoded integer to float64.
//
// The encoding is: real_price = value * 10^(priceType - 10).
// This matches the Rust Price::to_f64() and Go client.go priceToFloat.
func PriceToF64(value int32, priceType int32) float64 {
	if priceType == 0 || value == 0 {
		return 0.0
	}
	return float64(value) * math.Pow(10, float64(priceType-10))
}

// FpssEventKind identifies the type of an FPSS streaming event.
type FpssEventKind int

const (
	FpssQuoteEvent        FpssEventKind = 0
	FpssTradeEvent        FpssEventKind = 1
	FpssOpenInterestEvent FpssEventKind = 2
	FpssOhlcvcEvent       FpssEventKind = 3
	FpssControlEvent      FpssEventKind = 4
	FpssRawDataEvent      FpssEventKind = 5
)

// FpssQuote is a real-time quote event from FPSS.
// Bid and Ask are pre-decoded to float64. Use BidRaw/AskRaw for the original integers.
type FpssQuote struct {
	ContractID   int32
	MsOfDay      int32
	BidSize      int32
	BidExchange  int32
	Bid          float64
	BidRaw       int32
	BidCondition int32
	AskSize      int32
	AskExchange  int32
	Ask          float64
	AskRaw       int32
	AskCondition int32
	Date         int32
	ReceivedAtNs uint64
}

// FpssTrade is a real-time trade event from FPSS.
// Price is pre-decoded to float64. Use PriceRaw for the original integer.
type FpssTrade struct {
	ContractID     int32
	MsOfDay        int32
	Sequence       int32
	ExtCondition1  int32
	ExtCondition2  int32
	ExtCondition3  int32
	ExtCondition4  int32
	Condition      int32
	Size           int32
	Exchange       int32
	Price          float64
	PriceRaw       int32
	ConditionFlags int32
	PriceFlags     int32
	VolumeType     int32
	RecordsBack    int32
	Date           int32
	ReceivedAtNs   uint64
}

// FpssOpenInterestData is a real-time open interest event from FPSS.
type FpssOpenInterestData struct {
	ContractID   int32
	MsOfDay      int32
	OpenInterest int32
	Date         int32
	ReceivedAtNs uint64
}

// FpssOhlcvc is a real-time OHLCVC bar event from FPSS.
// Open/High/Low/Close are pre-decoded to float64. Use OpenRaw/HighRaw/LowRaw/CloseRaw for the original integers.
type FpssOhlcvc struct {
	ContractID   int32
	MsOfDay      int32
	Open         float64
	OpenRaw      int32
	High         float64
	HighRaw      int32
	Low          float64
	LowRaw       int32
	Close        float64
	CloseRaw     int32
	Volume       int64
	Count        int64
	Date         int32
	ReceivedAtNs uint64
}

// FpssControlData is a control/lifecycle event from FPSS.
//
// Kind encodes the sub-type:
//
//	0=login_success, 1=contract_assigned, 2=req_response,
//	3=market_open, 4=market_close, 5=server_error,
//	6=disconnected, 7=error, 8=unknown
//
// ID carries the contract_id or req_id where applicable (0 otherwise).
// Detail is a human-readable string (may be empty).
type FpssControlData struct {
	Kind   int32
	ID     int32
	Detail string
}

// FpssEvent is a tagged streaming event from FPSS.
// Check Kind to determine which field is valid.
type FpssEvent struct {
	Kind         FpssEventKind
	Quote        *FpssQuote
	Trade        *FpssTrade
	OpenInterest *FpssOpenInterestData
	Ohlcvc       *FpssOhlcvc
	Control      *FpssControlData
	RawCode      uint8
	RawPayload   []byte
}

// FpssClient wraps the FPSS real-time streaming handle.
type FpssClient struct {
	handle *C.TdxFpssHandle
}

// NewFpssClient connects to the FPSS streaming servers and returns a client.
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

func (f *FpssClient) fpssCall(rc C.int) (int, error) {
	if rc < 0 {
		return int(rc), fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// SubscribeQuotes subscribes to real-time quote data for a stock symbol.
func (f *FpssClient) SubscribeQuotes(symbol string) (int, error) {
	cs := C.CString(symbol)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_subscribe_quotes(f.handle, cs))
}

// SubscribeTrades subscribes to real-time trade data for a stock symbol.
func (f *FpssClient) SubscribeTrades(symbol string) (int, error) {
	cs := C.CString(symbol)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_subscribe_trades(f.handle, cs))
}

// SubscribeOpenInterest subscribes to open interest data for a stock symbol.
func (f *FpssClient) SubscribeOpenInterest(symbol string) (int, error) {
	cs := C.CString(symbol)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_subscribe_open_interest(f.handle, cs))
}

// SubscribeFullTrades subscribes to all trades for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) SubscribeFullTrades(secType string) (int, error) {
	cs := C.CString(secType)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_subscribe_full_trades(f.handle, cs))
}

// SubscribeFullOpenInterest subscribes to all open interest for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) SubscribeFullOpenInterest(secType string) (int, error) {
	cs := C.CString(secType)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_subscribe_full_open_interest(f.handle, cs))
}

// UnsubscribeQuotes unsubscribes from quote data for a stock symbol.
func (f *FpssClient) UnsubscribeQuotes(symbol string) (int, error) {
	cs := C.CString(symbol)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_unsubscribe_quotes(f.handle, cs))
}

// UnsubscribeTrades unsubscribes from trade data for a stock symbol.
func (f *FpssClient) UnsubscribeTrades(symbol string) (int, error) {
	cs := C.CString(symbol)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_unsubscribe_trades(f.handle, cs))
}

// UnsubscribeOpenInterest unsubscribes from open interest data for a stock symbol.
func (f *FpssClient) UnsubscribeOpenInterest(symbol string) (int, error) {
	cs := C.CString(symbol)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_unsubscribe_open_interest(f.handle, cs))
}

// UnsubscribeFullTrades unsubscribes from all trades for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) UnsubscribeFullTrades(secType string) (int, error) {
	cs := C.CString(secType)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_unsubscribe_full_trades(f.handle, cs))
}

// UnsubscribeFullOpenInterest unsubscribes from all open interest for a security type ("STOCK", "OPTION", "INDEX").
func (f *FpssClient) UnsubscribeFullOpenInterest(secType string) (int, error) {
	cs := C.CString(secType)
	defer C.free(unsafe.Pointer(cs))
	return f.fpssCall(C.tdx_fpss_unsubscribe_full_open_interest(f.handle, cs))
}

// IsAuthenticated returns true if the FPSS client is currently authenticated.
func (f *FpssClient) IsAuthenticated() bool {
	return C.tdx_fpss_is_authenticated(f.handle) != 0
}

// ContractLookup looks up a contract by its server-assigned ID.
func (f *FpssClient) ContractLookup(id int) (string, error) {
	cstr := C.tdx_fpss_contract_lookup(f.handle, C.int(id))
	if cstr == nil {
		return "", fmt.Errorf("thetadatadx: %s", lastError())
	}
	goStr := C.GoString(cstr)
	C.tdx_string_free(cstr)
	return goStr, nil
}

// Subscription represents a single active subscription entry.
type Subscription struct {
	Kind     string // "Quote", "Trade", or "OpenInterest"
	Contract string // "SPY" or "SPY 20260417 550 C"
}

// ActiveSubscriptions returns the currently active subscriptions as typed structs.
func (f *FpssClient) ActiveSubscriptions() ([]Subscription, error) {
	arr := C.tdx_fpss_active_subscriptions(f.handle)
	if arr == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	defer C.tdx_subscription_array_free(arr)
	n := int(arr.len)
	if n == 0 || arr.data == nil {
		return nil, nil
	}
	subs := unsafe.Slice(arr.data, n)
	result := make([]Subscription, n)
	for i := 0; i < n; i++ {
		if subs[i].kind != nil {
			result[i].Kind = C.GoString(subs[i].kind)
		}
		if subs[i].contract != nil {
			result[i].Contract = C.GoString(subs[i].contract)
		}
	}
	return result, nil
}

// NextEvent polls for the next streaming event with the given timeout in milliseconds.
// Returns nil if the timeout expires with no event.
func (f *FpssClient) NextEvent(timeoutMs uint64) (*FpssEvent, error) {
	raw := C.tdx_fpss_next_event(f.handle, C.uint64_t(timeoutMs))
	if raw == nil {
		return nil, nil
	}
	defer C.tdx_fpss_event_free(raw)

	event := &FpssEvent{
		Kind: FpssEventKind(raw.kind),
	}

	switch event.Kind {
	case FpssQuoteEvent:
		q := raw.quote
		event.Quote = &FpssQuote{
			ContractID:   int32(q.contract_id),
			MsOfDay:      int32(q.ms_of_day),
			BidSize:      int32(q.bid_size),
			BidExchange:  int32(q.bid_exchange),
			Bid:          float64(q.bid_f64),
			BidRaw:       int32(q.bid),
			BidCondition: int32(q.bid_condition),
			AskSize:      int32(q.ask_size),
			AskExchange:  int32(q.ask_exchange),
			Ask:          float64(q.ask_f64),
			AskRaw:       int32(q.ask),
			AskCondition: int32(q.ask_condition),
			Date:         int32(q.date),
			ReceivedAtNs: uint64(q.received_at_ns),
		}
	case FpssTradeEvent:
		t := raw.trade
		event.Trade = &FpssTrade{
			ContractID:     int32(t.contract_id),
			MsOfDay:        int32(t.ms_of_day),
			Sequence:       int32(t.sequence),
			ExtCondition1:  int32(t.ext_condition1),
			ExtCondition2:  int32(t.ext_condition2),
			ExtCondition3:  int32(t.ext_condition3),
			ExtCondition4:  int32(t.ext_condition4),
			Condition:      int32(t.condition),
			Size:           int32(t.size),
			Exchange:       int32(t.exchange),
			Price:          float64(t.price_f64),
			PriceRaw:       int32(t.price),
			ConditionFlags: int32(t.condition_flags),
			PriceFlags:     int32(t.price_flags),
			VolumeType:     int32(t.volume_type),
			RecordsBack:    int32(t.records_back),
			Date:           int32(t.date),
			ReceivedAtNs:   uint64(t.received_at_ns),
		}
	case FpssOpenInterestEvent:
		oi := raw.open_interest
		event.OpenInterest = &FpssOpenInterestData{
			ContractID:   int32(oi.contract_id),
			MsOfDay:      int32(oi.ms_of_day),
			OpenInterest: int32(oi.open_interest),
			Date:         int32(oi.date),
			ReceivedAtNs: uint64(oi.received_at_ns),
		}
	case FpssOhlcvcEvent:
		o := raw.ohlcvc
		event.Ohlcvc = &FpssOhlcvc{
			ContractID:   int32(o.contract_id),
			MsOfDay:      int32(o.ms_of_day),
			Open:         float64(o.open_f64),
			OpenRaw:      int32(o.open),
			High:         float64(o.high_f64),
			HighRaw:      int32(o.high),
			Low:          float64(o.low_f64),
			LowRaw:       int32(o.low),
			Close:        float64(o.close_f64),
			CloseRaw:     int32(o.close),
			Volume:       int64(o.volume),
			Count:        int64(o.count),
			Date:         int32(o.date),
			ReceivedAtNs: uint64(o.received_at_ns),
		}
	case FpssControlEvent:
		ctrl := raw.control
		detail := ""
		if ctrl.detail != nil {
			detail = C.GoString(ctrl.detail)
		}
		event.Control = &FpssControlData{
			Kind:   int32(ctrl.kind),
			ID:     int32(ctrl.id),
			Detail: detail,
		}
	case FpssRawDataEvent:
		rd := raw.raw_data
		event.RawCode = uint8(rd.code)
		if rd.payload != nil && rd.payload_len > 0 {
			event.RawPayload = C.GoBytes(unsafe.Pointer(rd.payload), C.int(rd.payload_len))
		}
	}

	return event, nil
}

// Shutdown gracefully shuts down the FPSS streaming connection.
func (f *FpssClient) Shutdown() {
	if f.handle != nil {
		C.tdx_fpss_shutdown(f.handle)
	}
}

// Close frees the FPSS handle. Call after Shutdown.
func (f *FpssClient) Close() {
	if f.handle != nil {
		C.tdx_fpss_free(f.handle)
		f.handle = nil
	}
}
