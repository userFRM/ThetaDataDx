package thetadatadx

import (
	"testing"
	"unsafe"
)

// TestFFIStructSizes verifies that every Go C-mirror struct is the exact same
// size as the Rust #[repr(C, align(64))] original. A mismatch here means
// unsafe.Slice would read garbage across the FFI boundary.
//
// Note: size equality alone does NOT catch same-size field swaps. The
// offset-aware TestTickFieldOffsets test below is the real SSOT safety net
// — keep both. FPSS mirror types need cgo to reach `C.TdxFpss*`, and Go
// forbids `import "C"` in `_test.go` files, so those offset checks live
// in `tick_ffi_mirrors.go`'s `init()` panic block (startup-time abort).
func TestFFIStructSizes(t *testing.T) {
	tests := []struct {
		name string
		got  uintptr
		want uintptr
	}{
		{"cCalendarDay", unsafe.Sizeof(cCalendarDay{}), 64},
		{"cEodTick", unsafe.Sizeof(cEodTick{}), 128},
		{"cOhlcTick", unsafe.Sizeof(cOhlcTick{}), 128},
		{"cTradeTick", unsafe.Sizeof(cTradeTick{}), 128},
		{"cQuoteTick", unsafe.Sizeof(cQuoteTick{}), 128},
		{"cOpenInterestTick", unsafe.Sizeof(cOpenInterestTick{}), 64},
		{"cInterestRateTick", unsafe.Sizeof(cInterestRateTick{}), 64},
		{"cIvTick", unsafe.Sizeof(cIvTick{}), 64},
		{"cPriceTick", unsafe.Sizeof(cPriceTick{}), 64},
		{"cMarketValueTick", unsafe.Sizeof(cMarketValueTick{}), 64},
		{"cGreeksTick", unsafe.Sizeof(cGreeksTick{}), 256},
		{"cTradeQuoteTick", unsafe.Sizeof(cTradeQuoteTick{}), 192},
	}
	for _, tt := range tests {
		if tt.got != tt.want {
			t.Errorf("%s: sizeof = %d, want %d (Rust ground truth)", tt.name, tt.got, tt.want)
		}
	}
}

// TestTickFieldOffsets verifies field offsets for every tick mirror struct
// against the Rust `#[repr(C, align(64))]` ground truth.
//
// Why offsets and not just sizes:
// A same-size field swap (e.g. reordering two int32 fields) keeps the total
// sizeof the same but silently corrupts every read. Sizeof alone can't catch
// that; offsetof comparisons can. The FPSS case that motivated adding this
// test — the C++ hand-written `TdxFpssEvent` ordered its Data fields as
// { quote, trade, open_interest, ohlcvc } while Rust emits
// { ohlcvc, open_interest, quote, trade }, so every C++ `event->quote.*`
// read belonged to a different variant at runtime. The FPSS mirror offset
// checks live in `tick_ffi_mirrors.go`'s `init()` block (cgo in `_test.go`
// is forbidden by the Go toolchain).
func TestTickFieldOffsets(t *testing.T) {
	tests := []struct {
		name string
		got  uintptr
		want uintptr
	}{
		// cEodTick
		{"cEodTick.MsOfDay", unsafe.Offsetof(cEodTick{}.MsOfDay), 0},
		{"cEodTick.MsOfDay2", unsafe.Offsetof(cEodTick{}.MsOfDay2), 4},
		{"cEodTick.Open", unsafe.Offsetof(cEodTick{}.Open), 8},
		{"cEodTick.High", unsafe.Offsetof(cEodTick{}.High), 16},
		{"cEodTick.Low", unsafe.Offsetof(cEodTick{}.Low), 24},
		{"cEodTick.Close", unsafe.Offsetof(cEodTick{}.Close), 32},
		{"cEodTick.Volume", unsafe.Offsetof(cEodTick{}.Volume), 40},
		{"cEodTick.Count", unsafe.Offsetof(cEodTick{}.Count), 48},
		{"cEodTick.BidSize", unsafe.Offsetof(cEodTick{}.BidSize), 56},
		{"cEodTick.BidExchange", unsafe.Offsetof(cEodTick{}.BidExchange), 60},
		{"cEodTick.Bid", unsafe.Offsetof(cEodTick{}.Bid), 64},
		{"cEodTick.BidCondition", unsafe.Offsetof(cEodTick{}.BidCondition), 72},
		{"cEodTick.AskSize", unsafe.Offsetof(cEodTick{}.AskSize), 76},
		{"cEodTick.AskExchange", unsafe.Offsetof(cEodTick{}.AskExchange), 80},
		{"cEodTick.Ask", unsafe.Offsetof(cEodTick{}.Ask), 88},
		{"cEodTick.AskCondition", unsafe.Offsetof(cEodTick{}.AskCondition), 96},
		{"cEodTick.Date", unsafe.Offsetof(cEodTick{}.Date), 100},
		{"cEodTick.Expiration", unsafe.Offsetof(cEodTick{}.Expiration), 104},
		{"cEodTick.Strike", unsafe.Offsetof(cEodTick{}.Strike), 112},
		{"cEodTick.Right", unsafe.Offsetof(cEodTick{}.Right), 120},

		// cOhlcTick
		{"cOhlcTick.MsOfDay", unsafe.Offsetof(cOhlcTick{}.MsOfDay), 0},
		{"cOhlcTick.Open", unsafe.Offsetof(cOhlcTick{}.Open), 8},
		{"cOhlcTick.High", unsafe.Offsetof(cOhlcTick{}.High), 16},
		{"cOhlcTick.Low", unsafe.Offsetof(cOhlcTick{}.Low), 24},
		{"cOhlcTick.Close", unsafe.Offsetof(cOhlcTick{}.Close), 32},
		{"cOhlcTick.Volume", unsafe.Offsetof(cOhlcTick{}.Volume), 40},
		{"cOhlcTick.Count", unsafe.Offsetof(cOhlcTick{}.Count), 48},
		{"cOhlcTick.Date", unsafe.Offsetof(cOhlcTick{}.Date), 56},
		{"cOhlcTick.Expiration", unsafe.Offsetof(cOhlcTick{}.Expiration), 60},
		{"cOhlcTick.Strike", unsafe.Offsetof(cOhlcTick{}.Strike), 64},
		{"cOhlcTick.Right", unsafe.Offsetof(cOhlcTick{}.Right), 72},

		// cTradeTick
		{"cTradeTick.MsOfDay", unsafe.Offsetof(cTradeTick{}.MsOfDay), 0},
		{"cTradeTick.Sequence", unsafe.Offsetof(cTradeTick{}.Sequence), 4},
		{"cTradeTick.ExtCondition1", unsafe.Offsetof(cTradeTick{}.ExtCondition1), 8},
		{"cTradeTick.ExtCondition2", unsafe.Offsetof(cTradeTick{}.ExtCondition2), 12},
		{"cTradeTick.ExtCondition3", unsafe.Offsetof(cTradeTick{}.ExtCondition3), 16},
		{"cTradeTick.ExtCondition4", unsafe.Offsetof(cTradeTick{}.ExtCondition4), 20},
		{"cTradeTick.Condition", unsafe.Offsetof(cTradeTick{}.Condition), 24},
		{"cTradeTick.Size", unsafe.Offsetof(cTradeTick{}.Size), 28},
		{"cTradeTick.Exchange", unsafe.Offsetof(cTradeTick{}.Exchange), 32},
		{"cTradeTick.Price", unsafe.Offsetof(cTradeTick{}.Price), 40},
		{"cTradeTick.ConditionFlags", unsafe.Offsetof(cTradeTick{}.ConditionFlags), 48},
		{"cTradeTick.PriceFlags", unsafe.Offsetof(cTradeTick{}.PriceFlags), 52},
		{"cTradeTick.VolumeType", unsafe.Offsetof(cTradeTick{}.VolumeType), 56},
		{"cTradeTick.RecordsBack", unsafe.Offsetof(cTradeTick{}.RecordsBack), 60},
		{"cTradeTick.Date", unsafe.Offsetof(cTradeTick{}.Date), 64},
		{"cTradeTick.Expiration", unsafe.Offsetof(cTradeTick{}.Expiration), 68},
		{"cTradeTick.Strike", unsafe.Offsetof(cTradeTick{}.Strike), 72},
		{"cTradeTick.Right", unsafe.Offsetof(cTradeTick{}.Right), 80},

		// cQuoteTick
		{"cQuoteTick.MsOfDay", unsafe.Offsetof(cQuoteTick{}.MsOfDay), 0},
		{"cQuoteTick.BidSize", unsafe.Offsetof(cQuoteTick{}.BidSize), 4},
		{"cQuoteTick.BidExchange", unsafe.Offsetof(cQuoteTick{}.BidExchange), 8},
		{"cQuoteTick.Bid", unsafe.Offsetof(cQuoteTick{}.Bid), 16},
		{"cQuoteTick.BidCondition", unsafe.Offsetof(cQuoteTick{}.BidCondition), 24},
		{"cQuoteTick.AskSize", unsafe.Offsetof(cQuoteTick{}.AskSize), 28},
		{"cQuoteTick.AskExchange", unsafe.Offsetof(cQuoteTick{}.AskExchange), 32},
		{"cQuoteTick.Ask", unsafe.Offsetof(cQuoteTick{}.Ask), 40},
		{"cQuoteTick.AskCondition", unsafe.Offsetof(cQuoteTick{}.AskCondition), 48},
		{"cQuoteTick.Date", unsafe.Offsetof(cQuoteTick{}.Date), 52},
		{"cQuoteTick.Expiration", unsafe.Offsetof(cQuoteTick{}.Expiration), 56},
		{"cQuoteTick.Strike", unsafe.Offsetof(cQuoteTick{}.Strike), 64},
		{"cQuoteTick.Right", unsafe.Offsetof(cQuoteTick{}.Right), 72},
		{"cQuoteTick.Midpoint", unsafe.Offsetof(cQuoteTick{}.Midpoint), 80},

		// cOpenInterestTick
		{"cOpenInterestTick.MsOfDay", unsafe.Offsetof(cOpenInterestTick{}.MsOfDay), 0},
		{"cOpenInterestTick.OpenInterest", unsafe.Offsetof(cOpenInterestTick{}.OpenInterest), 4},
		{"cOpenInterestTick.Date", unsafe.Offsetof(cOpenInterestTick{}.Date), 8},
		{"cOpenInterestTick.Expiration", unsafe.Offsetof(cOpenInterestTick{}.Expiration), 12},
		{"cOpenInterestTick.Strike", unsafe.Offsetof(cOpenInterestTick{}.Strike), 16},
		{"cOpenInterestTick.Right", unsafe.Offsetof(cOpenInterestTick{}.Right), 24},

		// cCalendarDay
		{"cCalendarDay.Date", unsafe.Offsetof(cCalendarDay{}.Date), 0},
		{"cCalendarDay.IsOpen", unsafe.Offsetof(cCalendarDay{}.IsOpen), 4},
		{"cCalendarDay.OpenTime", unsafe.Offsetof(cCalendarDay{}.OpenTime), 8},
		{"cCalendarDay.CloseTime", unsafe.Offsetof(cCalendarDay{}.CloseTime), 12},
		{"cCalendarDay.Status", unsafe.Offsetof(cCalendarDay{}.Status), 16},

		// cInterestRateTick
		{"cInterestRateTick.MsOfDay", unsafe.Offsetof(cInterestRateTick{}.MsOfDay), 0},
		{"cInterestRateTick.Rate", unsafe.Offsetof(cInterestRateTick{}.Rate), 8},
		{"cInterestRateTick.Date", unsafe.Offsetof(cInterestRateTick{}.Date), 16},

		// cIvTick
		{"cIvTick.MsOfDay", unsafe.Offsetof(cIvTick{}.MsOfDay), 0},
		{"cIvTick.ImpliedVolatility", unsafe.Offsetof(cIvTick{}.ImpliedVolatility), 8},
		{"cIvTick.IvError", unsafe.Offsetof(cIvTick{}.IvError), 16},
		{"cIvTick.Date", unsafe.Offsetof(cIvTick{}.Date), 24},
		{"cIvTick.Expiration", unsafe.Offsetof(cIvTick{}.Expiration), 28},
		{"cIvTick.Strike", unsafe.Offsetof(cIvTick{}.Strike), 32},
		{"cIvTick.Right", unsafe.Offsetof(cIvTick{}.Right), 40},

		// cPriceTick
		{"cPriceTick.MsOfDay", unsafe.Offsetof(cPriceTick{}.MsOfDay), 0},
		{"cPriceTick.Price", unsafe.Offsetof(cPriceTick{}.Price), 8},
		{"cPriceTick.Date", unsafe.Offsetof(cPriceTick{}.Date), 16},

		// cMarketValueTick
		{"cMarketValueTick.MsOfDay", unsafe.Offsetof(cMarketValueTick{}.MsOfDay), 0},
		{"cMarketValueTick.MarketBid", unsafe.Offsetof(cMarketValueTick{}.MarketBid), 8},
		{"cMarketValueTick.MarketAsk", unsafe.Offsetof(cMarketValueTick{}.MarketAsk), 16},
		{"cMarketValueTick.MarketPrice", unsafe.Offsetof(cMarketValueTick{}.MarketPrice), 24},
		{"cMarketValueTick.Date", unsafe.Offsetof(cMarketValueTick{}.Date), 32},
		{"cMarketValueTick.Expiration", unsafe.Offsetof(cMarketValueTick{}.Expiration), 36},
		{"cMarketValueTick.Strike", unsafe.Offsetof(cMarketValueTick{}.Strike), 40},
		{"cMarketValueTick.Right", unsafe.Offsetof(cMarketValueTick{}.Right), 48},

		// cGreeksTick
		{"cGreeksTick.MsOfDay", unsafe.Offsetof(cGreeksTick{}.MsOfDay), 0},
		{"cGreeksTick.ImpliedVolatility", unsafe.Offsetof(cGreeksTick{}.ImpliedVolatility), 8},
		{"cGreeksTick.Delta", unsafe.Offsetof(cGreeksTick{}.Delta), 16},
		{"cGreeksTick.Gamma", unsafe.Offsetof(cGreeksTick{}.Gamma), 24},
		{"cGreeksTick.Theta", unsafe.Offsetof(cGreeksTick{}.Theta), 32},
		{"cGreeksTick.Vega", unsafe.Offsetof(cGreeksTick{}.Vega), 40},
		{"cGreeksTick.Rho", unsafe.Offsetof(cGreeksTick{}.Rho), 48},
		{"cGreeksTick.IvError", unsafe.Offsetof(cGreeksTick{}.IvError), 56},
		{"cGreeksTick.Vanna", unsafe.Offsetof(cGreeksTick{}.Vanna), 64},
		{"cGreeksTick.Charm", unsafe.Offsetof(cGreeksTick{}.Charm), 72},
		{"cGreeksTick.Vomma", unsafe.Offsetof(cGreeksTick{}.Vomma), 80},
		{"cGreeksTick.Veta", unsafe.Offsetof(cGreeksTick{}.Veta), 88},
		{"cGreeksTick.Speed", unsafe.Offsetof(cGreeksTick{}.Speed), 96},
		{"cGreeksTick.Zomma", unsafe.Offsetof(cGreeksTick{}.Zomma), 104},
		{"cGreeksTick.Color", unsafe.Offsetof(cGreeksTick{}.Color), 112},
		{"cGreeksTick.Ultima", unsafe.Offsetof(cGreeksTick{}.Ultima), 120},
		{"cGreeksTick.D1", unsafe.Offsetof(cGreeksTick{}.D1), 128},
		{"cGreeksTick.D2", unsafe.Offsetof(cGreeksTick{}.D2), 136},
		{"cGreeksTick.DualDelta", unsafe.Offsetof(cGreeksTick{}.DualDelta), 144},
		{"cGreeksTick.DualGamma", unsafe.Offsetof(cGreeksTick{}.DualGamma), 152},
		{"cGreeksTick.Epsilon", unsafe.Offsetof(cGreeksTick{}.Epsilon), 160},
		{"cGreeksTick.Lambda", unsafe.Offsetof(cGreeksTick{}.Lambda), 168},
		{"cGreeksTick.Vera", unsafe.Offsetof(cGreeksTick{}.Vera), 176},
		{"cGreeksTick.Date", unsafe.Offsetof(cGreeksTick{}.Date), 184},
		{"cGreeksTick.Expiration", unsafe.Offsetof(cGreeksTick{}.Expiration), 188},
		{"cGreeksTick.Strike", unsafe.Offsetof(cGreeksTick{}.Strike), 192},
		{"cGreeksTick.Right", unsafe.Offsetof(cGreeksTick{}.Right), 200},

		// cTradeQuoteTick
		{"cTradeQuoteTick.MsOfDay", unsafe.Offsetof(cTradeQuoteTick{}.MsOfDay), 0},
		{"cTradeQuoteTick.Sequence", unsafe.Offsetof(cTradeQuoteTick{}.Sequence), 4},
		{"cTradeQuoteTick.ExtCondition1", unsafe.Offsetof(cTradeQuoteTick{}.ExtCondition1), 8},
		{"cTradeQuoteTick.ExtCondition2", unsafe.Offsetof(cTradeQuoteTick{}.ExtCondition2), 12},
		{"cTradeQuoteTick.ExtCondition3", unsafe.Offsetof(cTradeQuoteTick{}.ExtCondition3), 16},
		{"cTradeQuoteTick.ExtCondition4", unsafe.Offsetof(cTradeQuoteTick{}.ExtCondition4), 20},
		{"cTradeQuoteTick.Condition", unsafe.Offsetof(cTradeQuoteTick{}.Condition), 24},
		{"cTradeQuoteTick.Size", unsafe.Offsetof(cTradeQuoteTick{}.Size), 28},
		{"cTradeQuoteTick.Exchange", unsafe.Offsetof(cTradeQuoteTick{}.Exchange), 32},
		{"cTradeQuoteTick.Price", unsafe.Offsetof(cTradeQuoteTick{}.Price), 40},
		{"cTradeQuoteTick.ConditionFlags", unsafe.Offsetof(cTradeQuoteTick{}.ConditionFlags), 48},
		{"cTradeQuoteTick.PriceFlags", unsafe.Offsetof(cTradeQuoteTick{}.PriceFlags), 52},
		{"cTradeQuoteTick.VolumeType", unsafe.Offsetof(cTradeQuoteTick{}.VolumeType), 56},
		{"cTradeQuoteTick.RecordsBack", unsafe.Offsetof(cTradeQuoteTick{}.RecordsBack), 60},
		{"cTradeQuoteTick.QuoteMsOfDay", unsafe.Offsetof(cTradeQuoteTick{}.QuoteMsOfDay), 64},
		{"cTradeQuoteTick.BidSize", unsafe.Offsetof(cTradeQuoteTick{}.BidSize), 68},
		{"cTradeQuoteTick.BidExchange", unsafe.Offsetof(cTradeQuoteTick{}.BidExchange), 72},
		{"cTradeQuoteTick.Bid", unsafe.Offsetof(cTradeQuoteTick{}.Bid), 80},
		{"cTradeQuoteTick.BidCondition", unsafe.Offsetof(cTradeQuoteTick{}.BidCondition), 88},
		{"cTradeQuoteTick.AskSize", unsafe.Offsetof(cTradeQuoteTick{}.AskSize), 92},
		{"cTradeQuoteTick.AskExchange", unsafe.Offsetof(cTradeQuoteTick{}.AskExchange), 96},
		{"cTradeQuoteTick.Ask", unsafe.Offsetof(cTradeQuoteTick{}.Ask), 104},
		{"cTradeQuoteTick.AskCondition", unsafe.Offsetof(cTradeQuoteTick{}.AskCondition), 112},
		{"cTradeQuoteTick.Date", unsafe.Offsetof(cTradeQuoteTick{}.Date), 116},
		{"cTradeQuoteTick.Expiration", unsafe.Offsetof(cTradeQuoteTick{}.Expiration), 120},
		{"cTradeQuoteTick.Strike", unsafe.Offsetof(cTradeQuoteTick{}.Strike), 128},
		{"cTradeQuoteTick.Right", unsafe.Offsetof(cTradeQuoteTick{}.Right), 136},
	}
	for _, tt := range tests {
		if tt.got != tt.want {
			t.Errorf("%s: offsetof = %d, want %d (Rust ground truth)", tt.name, tt.got, tt.want)
		}
	}
}
