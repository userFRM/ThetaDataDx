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
// volume(4), count(4), bid_size(4), bid_exchange(4), bid(8), bid_condition(4),
// ask_size(4), ask_exchange(4), pad(4), ask(8), ask_condition(4), date(4),
// exp(4), pad(4), strike(8), right(4), pad(4) -> 128 total
type cEodTick struct {
	MsOfDay      int32
	MsOfDay2     int32
	Open         float64
	High         float64
	Low          float64
	Close        float64
	Volume       int32
	Count        int32
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
	_pad3        [128 - 116]byte
}

// cOhlcTick mirrors tdbe::OhlcTick #[repr(C, align(64))]
// Layout: ms_of_day(4), pad(4), open(8), high(8), low(8), close(8),
// volume(4), count(4), date(4), exp(4), strike(8), right(4), pad(4) = 72
// rounded to 128
type cOhlcTick struct {
	MsOfDay    int32
	_pad1      int32
	Open       float64
	High       float64
	Low        float64
	Close      float64
	Volume     int32
	Count      int32
	Date       int32
	Expiration int32
	Strike     float64
	Right      int32
	_pad2      [128 - 68]byte
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
// NOTE: These assertions verify total struct size but not individual field
// offsets. If fields are reordered in Rust but the total size stays the same,
// data will be silently corrupt. To catch reordering, add offsetof() checks
// or use cbindgen to generate the C header directly from Rust.
func init() {
	checks := []struct {
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
		// TdxEndpointRequestOptions: 27 builder-param fields + cross-cutting
		// timeout_ms (uint64) + has_timeout_ms (int32) + tail-padding to align
		// to the uint64 = 168 bytes on x86_64 / aarch64.
		{"TdxEndpointRequestOptions", unsafe.Sizeof(C.TdxEndpointRequestOptions{}), 168},
	}
	for _, c := range checks {
		if c.got != c.want {
			panic(fmt.Sprintf("thetadatadx: %s size mismatch: Go=%d, expected=%d (Rust FFI layout changed)", c.name, c.got, c.want))
		}
	}
}
