package thetadatadx

// C-compatible struct mirrors matching Rust #[repr(C, align(64))].
//
// These are Go equivalents with matching field layout for unsafe.Slice
// conversion across the FFI boundary. The align(64) means each struct
// occupies a multiple of 64 bytes.
//
// These mirror repr(C, align(64)) Rust structs. Update when tick_schema.toml changes.
// Generating exact padding is impractical because it depends on C ABI layout
// rules not encoded in tick_schema.toml. The init() size assertions below
// catch any drift at startup.

/*
#include "ffi_bridge.h"
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// cEodTick mirrors tdbe::EodTick #[repr(C, align(64))]
// Layout: ms_of_day(4), ms_of_day2(4), open(8), high(8), low(8), close(8),
// volume(8), count(8), bid_size(4), bid_exchange(4), bid(8), bid_condition(4),
// ask_size(4), ask_exchange(4), pad(4), ask(8), ask_condition(4), date(4),
// exp(4), pad(4), strike(8), right(4), pad(4) -> 128 total.
// volume/count widened to int64 for issue #372.
type cEodTick struct {
	MsOfDay      int32
	MsOfDay2     int32
	Open         float64
	High         float64
	Low          float64
	Close        float64
	Volume       int64
	Count        int64
	BidSize      int32
	BidExchange  int32
	Bid          float64
	BidCondition int32
	AskSize      int32
	AskExchange  int32
	_pad1        int32
	Ask          float64
	AskCondition int32
	Date         int32
	Expiration   int32
	_pad2        int32
	Strike       float64
	Right        int32
	_pad3        [128 - 124]byte
}

// cOhlcTick mirrors tdbe::OhlcTick #[repr(C, align(64))]
// Layout: ms_of_day(4), pad(4), open(8), high(8), low(8), close(8),
// volume(8), count(8), date(4), exp(4), strike(8), right(4), pad(52) = 76
// rounded to 128. volume/count widened to int64 for issue #372.
type cOhlcTick struct {
	MsOfDay    int32
	_pad1      int32
	Open       float64
	High       float64
	Low        float64
	Close      float64
	Volume     int64
	Count      int64
	Date       int32
	Expiration int32
	Strike     float64
	Right      int32
	_pad2      [128 - 76]byte
}

// cTradeTick mirrors tdbe::TradeTick #[repr(C, align(64))]
// Layout: 9*i32(36), pad(4), price(8), 4*i32(16), date(4), exp(4),
// strike(8), right(4), pad(4) = 88, rounded to 128
type cTradeTick struct {
	MsOfDay        int32
	Sequence       int32
	ExtCondition1  int32
	ExtCondition2  int32
	ExtCondition3  int32
	ExtCondition4  int32
	Condition      int32
	Size           int32
	Exchange       int32
	_pad1          int32
	Price          float64
	ConditionFlags int32
	PriceFlags     int32
	VolumeType     int32
	RecordsBack    int32
	Date           int32
	Expiration     int32
	Strike         float64
	Right          int32
	_pad2          [128 - 84]byte
}

// cQuoteTick mirrors tdbe::QuoteTick #[repr(C, align(64))]
// Layout: ms_of_day(4), bid_size(4), bid_exchange(4), pad(4), bid(8),
// bid_condition(4), ask_size(4), ask_exchange(4), pad(4), ask(8),
// ask_condition(4), date(4), exp(4), pad(4), strike(8), right(4), pad(4),
// midpoint(8) = 96, rounded to 128
type cQuoteTick struct {
	MsOfDay      int32
	BidSize      int32
	BidExchange  int32
	_pad1        int32
	Bid          float64
	BidCondition int32
	AskSize      int32
	AskExchange  int32
	_pad2        int32
	Ask          float64
	AskCondition int32
	Date         int32
	Expiration   int32
	_pad3        int32
	Strike       float64
	Right        int32
	_pad4        int32
	Midpoint     float64
	_pad5        [128 - 88]byte
}

// cOpenInterestTick mirrors tdbe::OpenInterestTick #[repr(C, align(64))]
// Layout: ms_of_day(4), oi(4), date(4), exp(4), strike(8), right(4), pad(4) = 32
type cOpenInterestTick struct {
	MsOfDay      int32
	OpenInterest int32
	Date         int32
	Expiration   int32
	Strike       float64
	Right        int32
	_pad         [64 - 28]byte
}

// cCalendarDay mirrors tdbe::CalendarDay #[repr(C, align(64))]
type cCalendarDay struct {
	Date      int32
	IsOpen    int32
	OpenTime  int32
	CloseTime int32
	Status    int32
	_pad      [64 - 5*4]byte
}

// cInterestRateTick mirrors tdbe::InterestRateTick #[repr(C, align(64))]
type cInterestRateTick struct {
	MsOfDay int32
	_pad1   int32
	Rate    float64
	Date    int32
	_pad2   [64 - 4 - 4 - 8 - 4]byte
}

// cIvTick mirrors tdbe::IvTick #[repr(C, align(64))]
// Layout: ms_of_day(4), pad(4), iv(8), iv_error(8), date(4), exp(4), strike(8), right(4), pad(4) = 48
type cIvTick struct {
	MsOfDay           int32
	_pad1             int32
	ImpliedVolatility float64
	IvError           float64
	Date              int32
	Expiration        int32
	Strike            float64
	Right             int32
	_pad2             [64 - 44]byte
}

// cPriceTick mirrors tdbe::PriceTick #[repr(C, align(64))]
// Layout: ms_of_day(4), pad(4), price(8), date(4), pad to 64
type cPriceTick struct {
	MsOfDay int32
	_pad1   int32
	Price   float64
	Date    int32
	_pad2   [64 - 4 - 4 - 8 - 4]byte
}

// cMarketValueTick mirrors tdbe::MarketValueTick #[repr(C, align(64))]
// Layout: ms_of_day(4), pad(4), 3*f64(24), date(4), exp(4), strike(8), right(4), pad(12) = 64
type cMarketValueTick struct {
	MsOfDay     int32
	_pad1       int32
	MarketBid   float64
	MarketAsk   float64
	MarketPrice float64
	Date        int32
	Expiration  int32
	Strike      float64
	Right       int32
	_pad2       [64 - 52]byte
}

// cGreeksTick mirrors tdbe::GreeksTick #[repr(C, align(64))]
// Layout: ms_of_day(4), pad(4), 22*f64(176), date(4), exp(4), strike(8), right(4), pad(4) = 208
// rounded to 256
type cGreeksTick struct {
	MsOfDay           int32
	_pad1             int32
	ImpliedVolatility float64
	Delta             float64
	Gamma             float64
	Theta             float64
	Vega              float64
	Rho               float64
	IvError           float64
	Vanna             float64
	Charm             float64
	Vomma             float64
	Veta              float64
	Speed             float64
	Zomma             float64
	Color             float64
	Ultima            float64
	D1                float64
	D2                float64
	DualDelta         float64
	DualGamma         float64
	Epsilon           float64
	Lambda            float64
	Vera              float64
	Date              int32
	Expiration        int32
	Strike            float64
	Right             int32
	_pad2             [256 - 204]byte
}

// cTradeQuoteTick mirrors tdbe::TradeQuoteTick #[repr(C, align(64))]
// Layout: 9*i32(36), pad(4), price(8), 4*i32(16), quote_ms(4), bid_size(4),
// bid_exchange(4), pad(4), bid(8), bid_condition(4), ask_size(4),
// ask_exchange(4), pad(4), ask(8), ask_condition(4), date(4), exp(4), pad(4),
// strike(8), right(4), pad(4) = 168, rounded to 192
type cTradeQuoteTick struct {
	MsOfDay        int32
	Sequence       int32
	ExtCondition1  int32
	ExtCondition2  int32
	ExtCondition3  int32
	ExtCondition4  int32
	Condition      int32
	Size           int32
	Exchange       int32
	_pad1          int32
	Price          float64
	ConditionFlags int32
	PriceFlags     int32
	VolumeType     int32
	RecordsBack    int32
	QuoteMsOfDay   int32
	BidSize        int32
	BidExchange    int32
	_pad2          int32
	Bid            float64
	BidCondition   int32
	AskSize        int32
	AskExchange    int32
	_pad3          int32
	Ask            float64
	AskCondition   int32
	Date           int32
	Expiration     int32
	_pad4          int32
	Strike         float64
	Right          int32
	_pad5          [192 - 140]byte
}

// cOptionContract mirrors TdxOptionContract from FFI
// Layout: root(8 ptr), exp(4), pad(4), strike(8), right(4), pad(4) = 32
type cOptionContract struct {
	Root       uintptr // *const c_char
	Expiration int32
	_pad1      int32
	Strike     float64
	Right      int32
	_pad2      int32
}

// ── FFI layout assertions ──
//
// These assertions verify that Go struct layouts match the Rust #[repr(C)]
// FFI structs. If a Rust struct changes (field added/removed/reordered),
// these will panic at import time rather than silently reading corrupt data.
// The expected sizes are validated against the Rust sizeof at PR review time.
//
// Companion offset checks:
//   - Go mirror structs  → TestTickFieldOffsets in ffi_layout_test.go
//   - C FPSS cgo structs → the `offsetChecks` block below (cgo + `_test.go`
//     don't mix, per Go toolchain restriction, so FPSS mirrors are
//     validated here at package-load time instead of `go test`).
//
// Size equality alone cannot catch same-size field swaps — e.g. the
// pre-fix C++ `TdxFpssEvent` had { quote, trade, open_interest, ohlcvc }
// while Rust emits { ohlcvc, open_interest, quote, trade }: each `event`
// struct was the same size on both sides, but every `event->quote.*`
// read in C++ was pulling from where `ohlcvc` lived in Rust memory. The
// offset checks below would have flagged that at program start.
func init() {
	sizeChecks := []struct {
		name string
		got  uintptr
		want uintptr
	}{
		{"cEodTick", unsafe.Sizeof(cEodTick{}), 128},
		{"cOhlcTick", unsafe.Sizeof(cOhlcTick{}), 128},
		{"cTradeTick", unsafe.Sizeof(cTradeTick{}), 128},
		{"cQuoteTick", unsafe.Sizeof(cQuoteTick{}), 128},
		{"cOpenInterestTick", unsafe.Sizeof(cOpenInterestTick{}), 64},
		{"cCalendarDay", unsafe.Sizeof(cCalendarDay{}), 64},
		{"cInterestRateTick", unsafe.Sizeof(cInterestRateTick{}), 64},
		{"cIvTick", unsafe.Sizeof(cIvTick{}), 64},
		{"cPriceTick", unsafe.Sizeof(cPriceTick{}), 64},
		{"cMarketValueTick", unsafe.Sizeof(cMarketValueTick{}), 64},
		{"cGreeksTick", unsafe.Sizeof(cGreeksTick{}), 256},
		{"cTradeQuoteTick", unsafe.Sizeof(cTradeQuoteTick{}), 192},
		{"cOptionContract", unsafe.Sizeof(cOptionContract{}), 32},
		// TdxEndpointRequestOptions: 29 builder-param fields (strike + right
		// joined the builder-bound set in v8.0.10 alongside the existing 27)
		// plus cross-cutting timeout_ms (uint64) + has_timeout_ms (int32) +
		// tail-padding to align to uint64 = 192 bytes on x86_64 / aarch64.
		{"TdxEndpointRequestOptions", unsafe.Sizeof(C.TdxEndpointRequestOptions{}), 192},
		// FPSS cgo structs — schema-driven from fpss_event_schema.toml,
		// same layout as ffi/src/fpss_event_structs.rs. Every data variant
		// carries an embedded TdxContract (32 bytes on LP64) immediately
		// after contract_id; sizes below recomputed from the struct under
		// -O2 on an LP64 host and cross-validated against the C++
		// static_assert block in sdks/cpp/include/thetadx.hpp.
		{"C.TdxFpssOhlcvc", unsafe.Sizeof(C.TdxFpssOhlcvc{}), 112},
		{"C.TdxFpssOpenInterest", unsafe.Sizeof(C.TdxFpssOpenInterest{}), 64},
		{"C.TdxFpssQuote", unsafe.Sizeof(C.TdxFpssQuote{}), 104},
		{"C.TdxFpssTrade", unsafe.Sizeof(C.TdxFpssTrade{}), 120},
		{"C.TdxFpssControl", unsafe.Sizeof(C.TdxFpssControl{}), 16},
		{"C.TdxFpssRawData", unsafe.Sizeof(C.TdxFpssRawData{}), 24},
		{"C.TdxFpssEvent", unsafe.Sizeof(C.TdxFpssEvent{}), 448},
	}
	for _, c := range sizeChecks {
		if c.got != c.want {
			panic(fmt.Sprintf("thetadatadx: %s size mismatch: Go=%d, expected=%d (Rust FFI layout changed)", c.name, c.got, c.want))
		}
	}

	// FPSS field-offset checks — mirrors the static_assert(offsetof) block
	// in sdks/cpp/include/thetadx.hpp. Field order in the wrapped Event
	// struct is { kind, ohlcvc, open_interest, quote, trade, control,
	// raw_data } per the Rust #[repr(C)] ground truth.
	offsetChecks := []struct {
		name string
		got  uintptr
		want uintptr
	}{
		// C.TdxFpssOhlcvc
		{"C.TdxFpssOhlcvc.contract_id", unsafe.Offsetof(C.TdxFpssOhlcvc{}.contract_id), 0},
		{"C.TdxFpssOhlcvc.contract", unsafe.Offsetof(C.TdxFpssOhlcvc{}.contract), 8},
		{"C.TdxFpssOhlcvc.ms_of_day", unsafe.Offsetof(C.TdxFpssOhlcvc{}.ms_of_day), 40},
		{"C.TdxFpssOhlcvc.open", unsafe.Offsetof(C.TdxFpssOhlcvc{}.open), 48},
		{"C.TdxFpssOhlcvc.high", unsafe.Offsetof(C.TdxFpssOhlcvc{}.high), 56},
		{"C.TdxFpssOhlcvc.low", unsafe.Offsetof(C.TdxFpssOhlcvc{}.low), 64},
		{"C.TdxFpssOhlcvc.close", unsafe.Offsetof(C.TdxFpssOhlcvc{}.close), 72},
		{"C.TdxFpssOhlcvc.volume", unsafe.Offsetof(C.TdxFpssOhlcvc{}.volume), 80},
		{"C.TdxFpssOhlcvc.count", unsafe.Offsetof(C.TdxFpssOhlcvc{}.count), 88},
		{"C.TdxFpssOhlcvc.date", unsafe.Offsetof(C.TdxFpssOhlcvc{}.date), 96},
		{"C.TdxFpssOhlcvc.received_at_ns", unsafe.Offsetof(C.TdxFpssOhlcvc{}.received_at_ns), 104},
		// C.TdxFpssOpenInterest
		{"C.TdxFpssOpenInterest.contract_id", unsafe.Offsetof(C.TdxFpssOpenInterest{}.contract_id), 0},
		{"C.TdxFpssOpenInterest.contract", unsafe.Offsetof(C.TdxFpssOpenInterest{}.contract), 8},
		{"C.TdxFpssOpenInterest.ms_of_day", unsafe.Offsetof(C.TdxFpssOpenInterest{}.ms_of_day), 40},
		{"C.TdxFpssOpenInterest.open_interest", unsafe.Offsetof(C.TdxFpssOpenInterest{}.open_interest), 44},
		{"C.TdxFpssOpenInterest.date", unsafe.Offsetof(C.TdxFpssOpenInterest{}.date), 48},
		{"C.TdxFpssOpenInterest.received_at_ns", unsafe.Offsetof(C.TdxFpssOpenInterest{}.received_at_ns), 56},
		// C.TdxFpssQuote
		{"C.TdxFpssQuote.contract_id", unsafe.Offsetof(C.TdxFpssQuote{}.contract_id), 0},
		{"C.TdxFpssQuote.contract", unsafe.Offsetof(C.TdxFpssQuote{}.contract), 8},
		{"C.TdxFpssQuote.ms_of_day", unsafe.Offsetof(C.TdxFpssQuote{}.ms_of_day), 40},
		{"C.TdxFpssQuote.bid_size", unsafe.Offsetof(C.TdxFpssQuote{}.bid_size), 44},
		{"C.TdxFpssQuote.bid_exchange", unsafe.Offsetof(C.TdxFpssQuote{}.bid_exchange), 48},
		{"C.TdxFpssQuote.bid", unsafe.Offsetof(C.TdxFpssQuote{}.bid), 56},
		{"C.TdxFpssQuote.bid_condition", unsafe.Offsetof(C.TdxFpssQuote{}.bid_condition), 64},
		{"C.TdxFpssQuote.ask_size", unsafe.Offsetof(C.TdxFpssQuote{}.ask_size), 68},
		{"C.TdxFpssQuote.ask_exchange", unsafe.Offsetof(C.TdxFpssQuote{}.ask_exchange), 72},
		{"C.TdxFpssQuote.ask", unsafe.Offsetof(C.TdxFpssQuote{}.ask), 80},
		{"C.TdxFpssQuote.ask_condition", unsafe.Offsetof(C.TdxFpssQuote{}.ask_condition), 88},
		{"C.TdxFpssQuote.date", unsafe.Offsetof(C.TdxFpssQuote{}.date), 92},
		{"C.TdxFpssQuote.received_at_ns", unsafe.Offsetof(C.TdxFpssQuote{}.received_at_ns), 96},
		// C.TdxFpssTrade
		{"C.TdxFpssTrade.contract_id", unsafe.Offsetof(C.TdxFpssTrade{}.contract_id), 0},
		{"C.TdxFpssTrade.contract", unsafe.Offsetof(C.TdxFpssTrade{}.contract), 8},
		{"C.TdxFpssTrade.ms_of_day", unsafe.Offsetof(C.TdxFpssTrade{}.ms_of_day), 40},
		{"C.TdxFpssTrade.sequence", unsafe.Offsetof(C.TdxFpssTrade{}.sequence), 44},
		{"C.TdxFpssTrade.ext_condition1", unsafe.Offsetof(C.TdxFpssTrade{}.ext_condition1), 48},
		{"C.TdxFpssTrade.ext_condition2", unsafe.Offsetof(C.TdxFpssTrade{}.ext_condition2), 52},
		{"C.TdxFpssTrade.ext_condition3", unsafe.Offsetof(C.TdxFpssTrade{}.ext_condition3), 56},
		{"C.TdxFpssTrade.ext_condition4", unsafe.Offsetof(C.TdxFpssTrade{}.ext_condition4), 60},
		{"C.TdxFpssTrade.condition", unsafe.Offsetof(C.TdxFpssTrade{}.condition), 64},
		{"C.TdxFpssTrade.size", unsafe.Offsetof(C.TdxFpssTrade{}.size), 68},
		{"C.TdxFpssTrade.exchange", unsafe.Offsetof(C.TdxFpssTrade{}.exchange), 72},
		{"C.TdxFpssTrade.price", unsafe.Offsetof(C.TdxFpssTrade{}.price), 80},
		{"C.TdxFpssTrade.condition_flags", unsafe.Offsetof(C.TdxFpssTrade{}.condition_flags), 88},
		{"C.TdxFpssTrade.price_flags", unsafe.Offsetof(C.TdxFpssTrade{}.price_flags), 92},
		{"C.TdxFpssTrade.volume_type", unsafe.Offsetof(C.TdxFpssTrade{}.volume_type), 96},
		{"C.TdxFpssTrade.records_back", unsafe.Offsetof(C.TdxFpssTrade{}.records_back), 100},
		{"C.TdxFpssTrade.date", unsafe.Offsetof(C.TdxFpssTrade{}.date), 104},
		{"C.TdxFpssTrade.received_at_ns", unsafe.Offsetof(C.TdxFpssTrade{}.received_at_ns), 112},
		// C.TdxFpssControl
		{"C.TdxFpssControl.kind", unsafe.Offsetof(C.TdxFpssControl{}.kind), 0},
		{"C.TdxFpssControl.id", unsafe.Offsetof(C.TdxFpssControl{}.id), 4},
		{"C.TdxFpssControl.detail", unsafe.Offsetof(C.TdxFpssControl{}.detail), 8},
		// C.TdxFpssRawData
		{"C.TdxFpssRawData.code", unsafe.Offsetof(C.TdxFpssRawData{}.code), 0},
		{"C.TdxFpssRawData.payload", unsafe.Offsetof(C.TdxFpssRawData{}.payload), 8},
		{"C.TdxFpssRawData.payload_len", unsafe.Offsetof(C.TdxFpssRawData{}.payload_len), 16},
		// C.TdxFpssEvent — the motivating case.
		{"C.TdxFpssEvent.kind", unsafe.Offsetof(C.TdxFpssEvent{}.kind), 0},
		{"C.TdxFpssEvent.ohlcvc", unsafe.Offsetof(C.TdxFpssEvent{}.ohlcvc), 8},
		{"C.TdxFpssEvent.open_interest", unsafe.Offsetof(C.TdxFpssEvent{}.open_interest), 120},
		{"C.TdxFpssEvent.quote", unsafe.Offsetof(C.TdxFpssEvent{}.quote), 184},
		{"C.TdxFpssEvent.trade", unsafe.Offsetof(C.TdxFpssEvent{}.trade), 288},
		{"C.TdxFpssEvent.control", unsafe.Offsetof(C.TdxFpssEvent{}.control), 408},
		{"C.TdxFpssEvent.raw_data", unsafe.Offsetof(C.TdxFpssEvent{}.raw_data), 424},
	}
	for _, c := range offsetChecks {
		if c.got != c.want {
			panic(fmt.Sprintf("thetadatadx: %s offset mismatch: Go=%d, expected=%d (Rust FFI layout changed)", c.name, c.got, c.want))
		}
	}
}
