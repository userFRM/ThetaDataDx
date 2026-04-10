package thetadatadx

/*
#include "ffi_bridge.h"
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// ── C-compatible struct mirrors (matching Rust #[repr(C, align(64))]) ──
// These are Go equivalents with matching field layout for unsafe.Slice conversion.
// The align(64) means each struct occupies a multiple of 64 bytes.
// Price fields are f64 (decoded during parsing). No price_type in public API.

// cEodTick mirrors tdbe::EodTick #[repr(C, align(64))]
// Layout: i32(4)+i32(4)+f64(8)*6+i32(4)*6+f64(8)*2+i32(4)+i32(4)+f64(8)+i32(4)
// = 8 + 48 + 24 + 16 + 4 + 4 + 8 + 4 = 116, pad(4) to align f64, total needs care
// Actually repr(C): ms_of_day(4), ms_of_day2(4), open(8), high(8), low(8), close(8),
// volume(4), count(4), bid_size(4), bid_exchange(4), bid(8), bid_condition(4), pad(4),
// ask_size(4), ask_exchange(4), ask(8), ask_condition(4), date(4), exp(4), pad(4),
// strike(8), right(4), pad(4) -> 128 total, fits in 128 (2*64)
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
// Layout: ms_of_day(4), pad(4), 5*i64(40), date(4), exp(4), strike(8), right(4), pad(4) = 72
type cMarketValueTick struct {
	MsOfDay           int32
	_pad1             int32
	MarketCap         int64
	SharesOutstanding int64
	EnterpriseValue   int64
	BookValue         int64
	FreeFloat         int64
	Date              int32
	Expiration        int32
	Strike            float64
	Right             int32
	_pad2             [128 - 68]byte
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
// Panic at init time if Go struct sizes diverge from the Rust repr(C, align(64)) layout.
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
		{"cMarketValueTick", unsafe.Sizeof(cMarketValueTick{}), 128},
		{"cGreeksTick", unsafe.Sizeof(cGreeksTick{}), 256},
		{"cTradeQuoteTick", unsafe.Sizeof(cTradeQuoteTick{}), 192},
		{"cOptionContract", unsafe.Sizeof(cOptionContract{}), 32},
	}
	for _, c := range checks {
		if c.got != c.want {
			panic(fmt.Sprintf("thetadatadx: %s size mismatch: Go=%d, expected=%d (Rust FFI layout changed)", c.name, c.got, c.want))
		}
	}
}

// ── Go tick types (public API) ──
// These are pure Go structs with decoded float prices for user convenience.

type EodTick struct {
	MsOfDay      int     `json:"ms_of_day"`
	MsOfDay2     int     `json:"ms_of_day2"`
	Open         float64 `json:"open"`
	High         float64 `json:"high"`
	Low          float64 `json:"low"`
	Close        float64 `json:"close"`
	Volume       int     `json:"volume"`
	Count        int     `json:"count"`
	BidSize      int     `json:"bid_size"`
	BidExchange  int     `json:"bid_exchange"`
	Bid          float64 `json:"bid"`
	BidCondition int     `json:"bid_condition"`
	AskSize      int     `json:"ask_size"`
	AskExchange  int     `json:"ask_exchange"`
	Ask          float64 `json:"ask"`
	AskCondition int     `json:"ask_condition"`
	Date         int     `json:"date"`
	Expiration   int32   `json:"expiration,omitempty"`
	Strike       float64 `json:"strike,omitempty"`
	Right        string  `json:"right,omitempty"`
}

type OhlcTick struct {
	MsOfDay    int     `json:"ms_of_day"`
	Open       float64 `json:"open"`
	High       float64 `json:"high"`
	Low        float64 `json:"low"`
	Close      float64 `json:"close"`
	Volume     int     `json:"volume"`
	Count      int     `json:"count"`
	Date       int     `json:"date"`
	Expiration int32   `json:"expiration,omitempty"`
	Strike     float64 `json:"strike,omitempty"`
	Right      string  `json:"right,omitempty"`
}

type TradeTick struct {
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
	Date           int     `json:"date"`
	Expiration     int32   `json:"expiration,omitempty"`
	Strike         float64 `json:"strike,omitempty"`
	Right          string  `json:"right,omitempty"`
}

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
	Midpoint     float64 `json:"midpoint"`
	Date         int     `json:"date"`
	Expiration   int32   `json:"expiration,omitempty"`
	Strike       float64 `json:"strike,omitempty"`
	Right        string  `json:"right,omitempty"`
}

type TradeQuoteTick struct {
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
	QuoteMsOfDay   int     `json:"quote_ms_of_day"`
	BidSize        int     `json:"bid_size"`
	BidExchange    int     `json:"bid_exchange"`
	Bid            float64 `json:"bid"`
	BidCondition   int     `json:"bid_condition"`
	AskSize        int     `json:"ask_size"`
	AskExchange    int     `json:"ask_exchange"`
	Ask            float64 `json:"ask"`
	AskCondition   int     `json:"ask_condition"`
	Date           int     `json:"date"`
	Expiration     int32   `json:"expiration,omitempty"`
	Strike         float64 `json:"strike,omitempty"`
	Right          string  `json:"right,omitempty"`
}

type OpenInterestTick struct {
	MsOfDay      int     `json:"ms_of_day"`
	OpenInterest int     `json:"open_interest"`
	Date         int     `json:"date"`
	Expiration   int32   `json:"expiration,omitempty"`
	Strike       float64 `json:"strike,omitempty"`
	Right        string  `json:"right,omitempty"`
}

type MarketValueTick struct {
	MsOfDay   int     `json:"ms_of_day"`
	MarketCap int64   `json:"market_cap"`
	SharesOut int64   `json:"shares_outstanding"`
	EntValue  int64   `json:"enterprise_value"`
	BookValue int64   `json:"book_value"`
	FreeFloat int64   `json:"free_float"`
	Date      int     `json:"date"`
	Expiration int32  `json:"expiration,omitempty"`
	Strike    float64 `json:"strike,omitempty"`
	Right     string  `json:"right,omitempty"`
}

type GreeksTick struct {
	MsOfDay        int     `json:"ms_of_day"`
	IV             float64 `json:"implied_volatility"`
	Delta          float64 `json:"delta"`
	Gamma          float64 `json:"gamma"`
	Theta          float64 `json:"theta"`
	Vega           float64 `json:"vega"`
	Rho            float64 `json:"rho"`
	IVError        float64 `json:"iv_error"`
	Vanna          float64 `json:"vanna"`
	Charm          float64 `json:"charm"`
	Vomma          float64 `json:"vomma"`
	Veta           float64 `json:"veta"`
	Speed          float64 `json:"speed"`
	Zomma          float64 `json:"zomma"`
	Color          float64 `json:"color"`
	Ultima         float64 `json:"ultima"`
	D1             float64 `json:"d1"`
	D2             float64 `json:"d2"`
	DualDelta      float64 `json:"dual_delta"`
	DualGamma      float64 `json:"dual_gamma"`
	Epsilon        float64 `json:"epsilon"`
	Lambda         float64 `json:"lambda"`
	Vera       float64 `json:"vera"`
	Date       int     `json:"date"`
	Expiration int32   `json:"expiration,omitempty"`
	Strike     float64 `json:"strike,omitempty"`
	Right      string  `json:"right,omitempty"`
}

type IVTick struct {
	MsOfDay    int     `json:"ms_of_day"`
	IV         float64 `json:"implied_volatility"`
	IVError    float64 `json:"iv_error"`
	Date       int     `json:"date"`
	Expiration int32   `json:"expiration,omitempty"`
	Strike     float64 `json:"strike,omitempty"`
	Right      string  `json:"right,omitempty"`
}

type PriceTick struct {
	MsOfDay int     `json:"ms_of_day"`
	Price   float64 `json:"price"`
	Date    int     `json:"date"`
}

type CalendarDay struct {
	Date      int  `json:"date"`
	IsOpen    bool `json:"is_open"`
	OpenTime  int  `json:"open_time"`
	CloseTime int  `json:"close_time"`
	Status    int  `json:"status"`
}

type InterestRateTick struct {
	MsOfDay int     `json:"ms_of_day"`
	Rate    float64 `json:"rate"`
	Date    int     `json:"date"`
}

type OptionContract struct {
	Root       string  `json:"root"`
	Expiration int     `json:"expiration"`
	Strike     float64 `json:"strike"`
	Right      string  `json:"right"`
}

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

// ── Right decoding ──

// RightStr converts the raw right code to a string.
// 67='C' (Call), 80='P' (Put), 0="" (not set).
func RightStr(code int32) string {
	switch code {
	case 67:
		return "C"
	case 80:
		return "P"
	default:
		return ""
	}
}

// ── Generic array conversion helpers ──

func convertEodTicks(arr C.TdxTickArray) []EodTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cEodTick)(arr.data), n)
	result := make([]EodTick, n)
	for i, t := range src {
		result[i] = EodTick{
			MsOfDay: int(t.MsOfDay), MsOfDay2: int(t.MsOfDay2), Date: int(t.Date),
			Volume: int(t.Volume), Count: int(t.Count),
			Open: t.Open, High: t.High, Low: t.Low, Close: t.Close,
			BidSize: int(t.BidSize), BidExchange: int(t.BidExchange),
			Bid: t.Bid, BidCondition: int(t.BidCondition),
			AskSize: int(t.AskSize), AskExchange: int(t.AskExchange),
			Ask: t.Ask, AskCondition: int(t.AskCondition),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertOhlcTicks(arr C.TdxTickArray) []OhlcTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cOhlcTick)(arr.data), n)
	result := make([]OhlcTick, n)
	for i, t := range src {
		result[i] = OhlcTick{
			MsOfDay: int(t.MsOfDay), Volume: int(t.Volume), Count: int(t.Count), Date: int(t.Date),
			Open: t.Open, High: t.High, Low: t.Low, Close: t.Close,
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertTradeTicks(arr C.TdxTickArray) []TradeTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cTradeTick)(arr.data), n)
	result := make([]TradeTick, n)
	for i, t := range src {
		result[i] = TradeTick{
			MsOfDay: int(t.MsOfDay), Sequence: int(t.Sequence),
			ExtCondition1: int(t.ExtCondition1), ExtCondition2: int(t.ExtCondition2),
			ExtCondition3: int(t.ExtCondition3), ExtCondition4: int(t.ExtCondition4),
			Condition: int(t.Condition),
			Size: int(t.Size), Exchange: int(t.Exchange), Price: t.Price,
			ConditionFlags: int(t.ConditionFlags),
			PriceFlags: int(t.PriceFlags), VolumeType: int(t.VolumeType), RecordsBack: int(t.RecordsBack),
			Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertQuoteTicks(arr C.TdxTickArray) []QuoteTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cQuoteTick)(arr.data), n)
	result := make([]QuoteTick, n)
	for i, t := range src {
		result[i] = QuoteTick{
			MsOfDay: int(t.MsOfDay), BidSize: int(t.BidSize), BidExchange: int(t.BidExchange),
			Bid: t.Bid, BidCondition: int(t.BidCondition),
			AskSize: int(t.AskSize), AskExchange: int(t.AskExchange),
			Ask: t.Ask, AskCondition: int(t.AskCondition),
			Midpoint: t.Midpoint, Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertTradeQuoteTicks(arr C.TdxTickArray) []TradeQuoteTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cTradeQuoteTick)(arr.data), n)
	result := make([]TradeQuoteTick, n)
	for i, t := range src {
		result[i] = TradeQuoteTick{
			MsOfDay: int(t.MsOfDay), Sequence: int(t.Sequence),
			ExtCondition1: int(t.ExtCondition1), ExtCondition2: int(t.ExtCondition2),
			ExtCondition3: int(t.ExtCondition3), ExtCondition4: int(t.ExtCondition4),
			Condition: int(t.Condition),
			Size: int(t.Size), Exchange: int(t.Exchange), Price: t.Price,
			ConditionFlags: int(t.ConditionFlags), PriceFlags: int(t.PriceFlags),
			VolumeType: int(t.VolumeType), RecordsBack: int(t.RecordsBack),
			QuoteMsOfDay: int(t.QuoteMsOfDay), BidSize: int(t.BidSize), BidExchange: int(t.BidExchange),
			Bid: t.Bid, BidCondition: int(t.BidCondition),
			AskSize: int(t.AskSize), AskExchange: int(t.AskExchange),
			Ask: t.Ask, AskCondition: int(t.AskCondition),
			Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertOpenInterestTicks(arr C.TdxTickArray) []OpenInterestTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cOpenInterestTick)(arr.data), n)
	result := make([]OpenInterestTick, n)
	for i, t := range src {
		result[i] = OpenInterestTick{
			MsOfDay: int(t.MsOfDay), OpenInterest: int(t.OpenInterest), Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertMarketValueTicks(arr C.TdxTickArray) []MarketValueTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cMarketValueTick)(arr.data), n)
	result := make([]MarketValueTick, n)
	for i, t := range src {
		result[i] = MarketValueTick{
			MsOfDay: int(t.MsOfDay), MarketCap: t.MarketCap, SharesOut: t.SharesOutstanding,
			EntValue: t.EnterpriseValue, BookValue: t.BookValue, FreeFloat: t.FreeFloat, Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertGreeksTicks(arr C.TdxTickArray) []GreeksTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cGreeksTick)(arr.data), n)
	result := make([]GreeksTick, n)
	for i, t := range src {
		result[i] = GreeksTick{
			MsOfDay: int(t.MsOfDay), IV: t.ImpliedVolatility, Delta: t.Delta, Gamma: t.Gamma,
			Theta: t.Theta, Vega: t.Vega, Rho: t.Rho, IVError: t.IvError,
			Vanna: t.Vanna, Charm: t.Charm, Vomma: t.Vomma, Veta: t.Veta,
			Speed: t.Speed, Zomma: t.Zomma, Color: t.Color, Ultima: t.Ultima,
			D1: t.D1, D2: t.D2, DualDelta: t.DualDelta, DualGamma: t.DualGamma,
			Epsilon: t.Epsilon, Lambda: t.Lambda, Vera: t.Vera, Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertIvTicks(arr C.TdxTickArray) []IVTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cIvTick)(arr.data), n)
	result := make([]IVTick, n)
	for i, t := range src {
		result[i] = IVTick{
			MsOfDay: int(t.MsOfDay), IV: t.ImpliedVolatility, IVError: t.IvError, Date: int(t.Date),
			Expiration: t.Expiration, Strike: t.Strike, Right: RightStr(t.Right),
		}
	}
	return result
}

func convertPriceTicks(arr C.TdxTickArray) []PriceTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cPriceTick)(arr.data), n)
	result := make([]PriceTick, n)
	for i, t := range src {
		result[i] = PriceTick{MsOfDay: int(t.MsOfDay), Price: t.Price, Date: int(t.Date)}
	}
	return result
}

func convertCalendarDays(arr C.TdxTickArray) []CalendarDay {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cCalendarDay)(arr.data), n)
	result := make([]CalendarDay, n)
	for i, t := range src { result[i] = CalendarDay{int(t.Date), t.IsOpen != 0, int(t.OpenTime), int(t.CloseTime), int(t.Status)} }
	return result
}

func convertInterestRateTicks(arr C.TdxTickArray) []InterestRateTick {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cInterestRateTick)(arr.data), n)
	result := make([]InterestRateTick, n)
	for i, t := range src { result[i] = InterestRateTick{int(t.MsOfDay), t.Rate, int(t.Date)} }
	return result
}

func convertOptionContracts(arr C.TdxOptionContractArray) []OptionContract {
	if arr.data == nil || arr.len == 0 { return nil }
	n := int(arr.len)
	src := unsafe.Slice((*cOptionContract)(arr.data), n)
	result := make([]OptionContract, n)
	for i, t := range src {
		root := ""
		if t.Root != 0 {
			root = C.GoString((*C.char)(unsafe.Pointer(t.Root)))
		}
		result[i] = OptionContract{Root: root, Expiration: int(t.Expiration), Strike: t.Strike, Right: RightStr(t.Right)}
	}
	return result
}

// ── Client ──

type Client struct {
	handle *C.TdxClient
}

func Connect(creds *Credentials, config *Config) (*Client, error) {
	if creds == nil || creds.handle == nil { return nil, fmt.Errorf("thetadatadx: credentials handle is nil") }
	if config == nil || config.handle == nil { return nil, fmt.Errorf("thetadatadx: config handle is nil") }
	h := C.tdx_client_connect(creds.handle, config.handle)
	if h == nil { return nil, fmt.Errorf("thetadatadx: %s", lastError()) }
	return &Client{handle: h}, nil
}

func (c *Client) Close() {
	if c.handle != nil { C.tdx_client_free(c.handle); c.handle = nil }
}

// ═══════════════════════════════════════════════════════════════
//  Stock endpoints
// ═══════════════════════════════════════════════════════════════

func (c *Client) StockListSymbols() ([]string, error) { return stringArrayToGo(C.tdx_stock_list_symbols(c.handle)) }

func (c *Client) StockListDates(requestType, symbol string) ([]string, error) {
	cRT := C.CString(requestType); cSym := C.CString(symbol)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cSym))
	return stringArrayToGo(C.tdx_stock_list_dates(c.handle, cRT, cSym))
}

func (c *Client) StockSnapshotOHLC(symbols []string) ([]OhlcTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_stock_snapshot_ohlc(c.handle, cSyms, cLen); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}

func (c *Client) StockSnapshotTrade(symbols []string) ([]TradeTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_stock_snapshot_trade(c.handle, cSyms, cLen); result := convertTradeTicks(arr); C.tdx_trade_tick_array_free(arr); return result, nil
}

func (c *Client) StockSnapshotQuote(symbols []string) ([]QuoteTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_stock_snapshot_quote(c.handle, cSyms, cLen); result := convertQuoteTicks(arr); C.tdx_quote_tick_array_free(arr); return result, nil
}

func (c *Client) StockSnapshotMarketValue(symbols []string) ([]MarketValueTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_stock_snapshot_market_value(c.handle, cSyms, cLen); result := convertMarketValueTicks(arr); C.tdx_market_value_tick_array_free(arr); return result, nil
}

func (c *Client) StockHistoryEOD(symbol, startDate, endDate string) ([]EodTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	arr := C.tdx_stock_history_eod(c.handle, cS, cSt, cEn); result := convertEodTicks(arr); C.tdx_eod_tick_array_free(arr); return result, nil
}

func (c *Client) StockHistoryOHLC(symbol, date, interval string) ([]OhlcTick, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_stock_history_ohlc(c.handle, cS, cD, cI); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}

func (c *Client) StockHistoryOHLCRange(symbol, startDate, endDate, interval string) ([]OhlcTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_stock_history_ohlc_range(c.handle, cS, cSt, cEn, cI); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}

func (c *Client) StockHistoryTrade(symbol, date string) ([]TradeTick, error) {
	cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_stock_history_trade(c.handle, cS, cD); result := convertTradeTicks(arr); C.tdx_trade_tick_array_free(arr); return result, nil
}

func (c *Client) StockHistoryQuote(symbol, date, interval string) ([]QuoteTick, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_stock_history_quote(c.handle, cS, cD, cI); result := convertQuoteTicks(arr); C.tdx_quote_tick_array_free(arr); return result, nil
}

func (c *Client) StockHistoryTradeQuote(symbol, date string) ([]TradeQuoteTick, error) {
	cS := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_stock_history_trade_quote(c.handle, cS, cD); result := convertTradeQuoteTicks(arr); C.tdx_trade_quote_tick_array_free(arr); return result, nil
}

func (c *Client) StockAtTimeTrade(symbol, startDate, endDate, timeOfDay string) ([]TradeTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	arr := C.tdx_stock_at_time_trade(c.handle, cS, cSt, cEn, cT); result := convertTradeTicks(arr); C.tdx_trade_tick_array_free(arr); return result, nil
}

func (c *Client) StockAtTimeQuote(symbol, startDate, endDate, timeOfDay string) ([]QuoteTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	arr := C.tdx_stock_at_time_quote(c.handle, cS, cSt, cEn, cT); result := convertQuoteTicks(arr); C.tdx_quote_tick_array_free(arr); return result, nil
}

// ═══════════════════════════════════════════════════════════════
//  Option — List endpoints
// ═══════════════════════════════════════════════════════════════

func (c *Client) OptionListSymbols() ([]string, error) { return stringArrayToGo(C.tdx_option_list_symbols(c.handle)) }

func (c *Client) OptionListDates(requestType, symbol, expiration, strike, right string) ([]string, error) {
	cRT := C.CString(requestType); cSym := C.CString(symbol); cExp := C.CString(expiration); cStr := C.CString(strike); cR := C.CString(right)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp)); defer C.free(unsafe.Pointer(cStr)); defer C.free(unsafe.Pointer(cR))
	return stringArrayToGo(C.tdx_option_list_dates(c.handle, cRT, cSym, cExp, cStr, cR))
}

func (c *Client) OptionListExpirations(symbol string) ([]string, error) {
	cSym := C.CString(symbol); defer C.free(unsafe.Pointer(cSym))
	return stringArrayToGo(C.tdx_option_list_expirations(c.handle, cSym))
}

func (c *Client) OptionListStrikes(symbol, expiration string) ([]string, error) {
	cSym := C.CString(symbol); cExp := C.CString(expiration)
	defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cExp))
	return stringArrayToGo(C.tdx_option_list_strikes(c.handle, cSym, cExp))
}

func (c *Client) OptionListContracts(requestType, symbol, date string) ([]OptionContract, error) {
	cRT := C.CString(requestType); cSym := C.CString(symbol); cD := C.CString(date)
	defer C.free(unsafe.Pointer(cRT)); defer C.free(unsafe.Pointer(cSym)); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_list_contracts(c.handle, cRT, cSym, cD)
	result := convertOptionContracts(arr); C.tdx_option_contract_array_free(arr); return result, nil
}

// ═══════════════════════════════════════════════════════════════
//  Option — Snapshot, History, At-Time (abbreviated for brevity)
//  Pattern: C string args -> FFI call -> convert -> free -> return
// ═══════════════════════════════════════════════════════════════

// Helper for 4-param option endpoints (symbol, expiration, strike, right)
func (c *Client) optArgs4(s, e, k, r string) (*C.char, *C.char, *C.char, *C.char, func()) {
	cS := C.CString(s); cE := C.CString(e); cK := C.CString(k); cR := C.CString(r)
	return cS, cE, cK, cR, func() {
		C.free(unsafe.Pointer(cS)); C.free(unsafe.Pointer(cE)); C.free(unsafe.Pointer(cK)); C.free(unsafe.Pointer(cR))
	}
}

func (c *Client) OptionSnapshotOHLC(s, e, k, r string) ([]OhlcTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_ohlc(c.handle, cS, cE, cK, cR); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotTrade(s, e, k, r string) ([]TradeTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_trade(c.handle, cS, cE, cK, cR); result := convertTradeTicks(arr); C.tdx_trade_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotQuote(s, e, k, r string) ([]QuoteTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_quote(c.handle, cS, cE, cK, cR); result := convertQuoteTicks(arr); C.tdx_quote_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotOpenInterest(s, e, k, r string) ([]OpenInterestTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_open_interest(c.handle, cS, cE, cK, cR); result := convertOpenInterestTicks(arr); C.tdx_open_interest_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotMarketValue(s, e, k, r string) ([]MarketValueTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_market_value(c.handle, cS, cE, cK, cR); result := convertMarketValueTicks(arr); C.tdx_market_value_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotGreeksIV(s, e, k, r string) ([]IVTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_greeks_implied_volatility(c.handle, cS, cE, cK, cR); result := convertIvTicks(arr); C.tdx_iv_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotGreeksAll(s, e, k, r string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_greeks_all(c.handle, cS, cE, cK, cR); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotGreeksFirstOrder(s, e, k, r string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_greeks_first_order(c.handle, cS, cE, cK, cR); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotGreeksSecondOrder(s, e, k, r string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_greeks_second_order(c.handle, cS, cE, cK, cR); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionSnapshotGreeksThirdOrder(s, e, k, r string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	arr := C.tdx_option_snapshot_greeks_third_order(c.handle, cS, cE, cK, cR); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}

// Option history endpoints follow the same pattern. Including all for completeness.

func (c *Client) OptionHistoryEOD(s, e, k, r, sd, ed string) ([]EodTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cSD := C.CString(sd); cED := C.CString(ed); defer C.free(unsafe.Pointer(cSD)); defer C.free(unsafe.Pointer(cED))
	arr := C.tdx_option_history_eod(c.handle, cS, cE, cK, cR, cSD, cED); result := convertEodTicks(arr); C.tdx_eod_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryOHLC(s, e, k, r, d, iv string) ([]OhlcTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_ohlc(c.handle, cS, cE, cK, cR, cD, cI); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTrade(s, e, k, r, d string) ([]TradeTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade(c.handle, cS, cE, cK, cR, cD); result := convertTradeTicks(arr); C.tdx_trade_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryQuote(s, e, k, r, d, iv string) ([]QuoteTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_quote(c.handle, cS, cE, cK, cR, cD, cI); result := convertQuoteTicks(arr); C.tdx_quote_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTradeQuote(s, e, k, r, d string) ([]TradeQuoteTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade_quote(c.handle, cS, cE, cK, cR, cD); result := convertTradeQuoteTicks(arr); C.tdx_trade_quote_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryOpenInterest(s, e, k, r, d string) ([]OpenInterestTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_open_interest(c.handle, cS, cE, cK, cR, cD); result := convertOpenInterestTicks(arr); C.tdx_open_interest_tick_array_free(arr); return result, nil
}

// Option Greeks history (all 11 endpoints)
func (c *Client) OptionHistoryGreeksEOD(s, e, k, r, sd, ed string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cSD := C.CString(sd); cED := C.CString(ed); defer C.free(unsafe.Pointer(cSD)); defer C.free(unsafe.Pointer(cED))
	arr := C.tdx_option_history_greeks_eod(c.handle, cS, cE, cK, cR, cSD, cED); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryGreeksAll(s, e, k, r, d, iv string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_greeks_all(c.handle, cS, cE, cK, cR, cD, cI); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTradeGreeksAll(s, e, k, r, d string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade_greeks_all(c.handle, cS, cE, cK, cR, cD); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryGreeksFirstOrder(s, e, k, r, d, iv string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_greeks_first_order(c.handle, cS, cE, cK, cR, cD, cI); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTradeGreeksFirstOrder(s, e, k, r, d string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade_greeks_first_order(c.handle, cS, cE, cK, cR, cD); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryGreeksSecondOrder(s, e, k, r, d, iv string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_greeks_second_order(c.handle, cS, cE, cK, cR, cD, cI); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTradeGreeksSecondOrder(s, e, k, r, d string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade_greeks_second_order(c.handle, cS, cE, cK, cR, cD); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryGreeksThirdOrder(s, e, k, r, d, iv string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_greeks_third_order(c.handle, cS, cE, cK, cR, cD, cI); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTradeGreeksThirdOrder(s, e, k, r, d string) ([]GreeksTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade_greeks_third_order(c.handle, cS, cE, cK, cR, cD); result := convertGreeksTicks(arr); C.tdx_greeks_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryGreeksIV(s, e, k, r, d, iv string) ([]IVTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); cI := C.CString(iv); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_option_history_greeks_implied_volatility(c.handle, cS, cE, cK, cR, cD, cI); result := convertIvTicks(arr); C.tdx_iv_tick_array_free(arr); return result, nil
}
func (c *Client) OptionHistoryTradeGreeksIV(s, e, k, r, d string) ([]IVTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cD := C.CString(d); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_option_history_trade_greeks_implied_volatility(c.handle, cS, cE, cK, cR, cD); result := convertIvTicks(arr); C.tdx_iv_tick_array_free(arr); return result, nil
}

func (c *Client) OptionAtTimeTrade(s, e, k, r, sd, ed, tod string) ([]TradeTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cSD := C.CString(sd); cED := C.CString(ed); cT := C.CString(tod)
	defer C.free(unsafe.Pointer(cSD)); defer C.free(unsafe.Pointer(cED)); defer C.free(unsafe.Pointer(cT))
	arr := C.tdx_option_at_time_trade(c.handle, cS, cE, cK, cR, cSD, cED, cT); result := convertTradeTicks(arr); C.tdx_trade_tick_array_free(arr); return result, nil
}
func (c *Client) OptionAtTimeQuote(s, e, k, r, sd, ed, tod string) ([]QuoteTick, error) {
	cS, cE, cK, cR, free := c.optArgs4(s, e, k, r); defer free()
	cSD := C.CString(sd); cED := C.CString(ed); cT := C.CString(tod)
	defer C.free(unsafe.Pointer(cSD)); defer C.free(unsafe.Pointer(cED)); defer C.free(unsafe.Pointer(cT))
	arr := C.tdx_option_at_time_quote(c.handle, cS, cE, cK, cR, cSD, cED, cT); result := convertQuoteTicks(arr); C.tdx_quote_tick_array_free(arr); return result, nil
}

// ═══════════════════════════════════════════════════════════════
//  Index endpoints
// ═══════════════════════════════════════════════════════════════

func (c *Client) IndexListSymbols() ([]string, error) { return stringArrayToGo(C.tdx_index_list_symbols(c.handle)) }
func (c *Client) IndexListDates(symbol string) ([]string, error) {
	cSym := C.CString(symbol); defer C.free(unsafe.Pointer(cSym))
	return stringArrayToGo(C.tdx_index_list_dates(c.handle, cSym))
}

func (c *Client) IndexSnapshotOHLC(symbols []string) ([]OhlcTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_index_snapshot_ohlc(c.handle, cSyms, cLen); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}
func (c *Client) IndexSnapshotPrice(symbols []string) ([]PriceTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_index_snapshot_price(c.handle, cSyms, cLen); result := convertPriceTicks(arr); C.tdx_price_tick_array_free(arr); return result, nil
}
func (c *Client) IndexSnapshotMarketValue(symbols []string) ([]MarketValueTick, error) {
	cSyms, cLen := symbolsToCArray(symbols); defer freeSymbolArray(cSyms, cLen)
	arr := C.tdx_index_snapshot_market_value(c.handle, cSyms, cLen); result := convertMarketValueTicks(arr); C.tdx_market_value_tick_array_free(arr); return result, nil
}

func (c *Client) IndexHistoryEOD(symbol, startDate, endDate string) ([]EodTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	arr := C.tdx_index_history_eod(c.handle, cS, cSt, cEn); result := convertEodTicks(arr); C.tdx_eod_tick_array_free(arr); return result, nil
}
func (c *Client) IndexHistoryOHLC(symbol, startDate, endDate, interval string) ([]OhlcTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_index_history_ohlc(c.handle, cS, cSt, cEn, cI); result := convertOhlcTicks(arr); C.tdx_ohlc_tick_array_free(arr); return result, nil
}
func (c *Client) IndexHistoryPrice(symbol, date, interval string) ([]PriceTick, error) {
	cS := C.CString(symbol); cD := C.CString(date); cI := C.CString(interval)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cD)); defer C.free(unsafe.Pointer(cI))
	arr := C.tdx_index_history_price(c.handle, cS, cD, cI); result := convertPriceTicks(arr); C.tdx_price_tick_array_free(arr); return result, nil
}
func (c *Client) IndexAtTimePrice(symbol, startDate, endDate, timeOfDay string) ([]PriceTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate); cT := C.CString(timeOfDay)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn)); defer C.free(unsafe.Pointer(cT))
	arr := C.tdx_index_at_time_price(c.handle, cS, cSt, cEn, cT); result := convertPriceTicks(arr); C.tdx_price_tick_array_free(arr); return result, nil
}

// ═══════════════════════════════════════════════════════════════
//  Calendar + Interest Rate
// ═══════════════════════════════════════════════════════════════

func (c *Client) CalendarOpenToday() ([]CalendarDay, error) {
	arr := C.tdx_calendar_open_today(c.handle); result := convertCalendarDays(arr); C.tdx_calendar_day_array_free(arr); return result, nil
}
func (c *Client) CalendarOnDate(date string) ([]CalendarDay, error) {
	cD := C.CString(date); defer C.free(unsafe.Pointer(cD))
	arr := C.tdx_calendar_on_date(c.handle, cD); result := convertCalendarDays(arr); C.tdx_calendar_day_array_free(arr); return result, nil
}
func (c *Client) CalendarYear(year string) ([]CalendarDay, error) {
	cY := C.CString(year); defer C.free(unsafe.Pointer(cY))
	arr := C.tdx_calendar_year(c.handle, cY); result := convertCalendarDays(arr); C.tdx_calendar_day_array_free(arr); return result, nil
}
func (c *Client) InterestRateHistoryEOD(symbol, startDate, endDate string) ([]InterestRateTick, error) {
	cS := C.CString(symbol); cSt := C.CString(startDate); cEn := C.CString(endDate)
	defer C.free(unsafe.Pointer(cS)); defer C.free(unsafe.Pointer(cSt)); defer C.free(unsafe.Pointer(cEn))
	arr := C.tdx_interest_rate_history_eod(c.handle, cS, cSt, cEn); result := convertInterestRateTicks(arr); C.tdx_interest_rate_tick_array_free(arr); return result, nil
}

// ═══════════════════════════════════════════════════════════════
//  Greeks (standalone — typed struct, no JSON)
// ═══════════════════════════════════════════════════════════════

func AllGreeks(spot, strike, rate, divYield, tte, optionPrice float64, isCall bool) (*Greeks, error) {
	call := C.int(0); if isCall { call = 1 }
	ptr := C.tdx_all_greeks(C.double(spot), C.double(strike), C.double(rate), C.double(divYield), C.double(tte), C.double(optionPrice), call)
	if ptr == nil { return nil, fmt.Errorf("thetadatadx: %s", lastError()) }
	defer C.tdx_greeks_result_free(ptr)
	return &Greeks{
		Value:     float64(ptr.value),
		Delta:     float64(ptr.delta),
		Gamma:     float64(ptr.gamma),
		Theta:     float64(ptr.theta),
		Vega:      float64(ptr.vega),
		Rho:       float64(ptr.rho),
		IV:        float64(ptr.iv),
		IVError:   float64(ptr.iv_error),
		Vanna:     float64(ptr.vanna),
		Charm:     float64(ptr.charm),
		Vomma:     float64(ptr.vomma),
		Veta:      float64(ptr.veta),
		Speed:     float64(ptr.speed),
		Zomma:     float64(ptr.zomma),
		Color:     float64(ptr.color),
		Ultima:    float64(ptr.ultima),
		D1:        float64(ptr.d1),
		D2:        float64(ptr.d2),
		DualDelta: float64(ptr.dual_delta),
		DualGamma: float64(ptr.dual_gamma),
		Epsilon:   float64(ptr.epsilon),
		Lambda:    float64(ptr.lambda),
	}, nil
}

func ImpliedVolatility(spot, strike, rate, divYield, tte, optionPrice float64, isCall bool) (float64, float64, error) {
	call := C.int(0); if isCall { call = 1 }
	var iv, ivErr C.double
	rc := C.tdx_implied_volatility(C.double(spot), C.double(strike), C.double(rate), C.double(divYield), C.double(tte), C.double(optionPrice), call, &iv, &ivErr)
	if rc != 0 { return 0, 0, fmt.Errorf("thetadatadx: %s", lastError()) }
	return float64(iv), float64(ivErr), nil
}

// Suppress unused import warnings
var _ = unsafe.Pointer(nil)
