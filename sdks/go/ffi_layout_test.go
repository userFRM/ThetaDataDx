package thetadatadx

import (
	"testing"
	"unsafe"
)

// TestFFIStructSizes verifies that every Go C-mirror struct is the exact same
// size as the Rust #[repr(C, align(64))] original. A mismatch here means
// unsafe.Slice would read garbage across the FFI boundary.
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
		{"cMarketValueTick", unsafe.Sizeof(cMarketValueTick{}), 128},
		{"cGreeksTick", unsafe.Sizeof(cGreeksTick{}), 256},
		{"cTradeQuoteTick", unsafe.Sizeof(cTradeQuoteTick{}), 192},
	}
	for _, tt := range tests {
		if tt.got != tt.want {
			t.Errorf("%s: sizeof = %d, want %d (Rust ground truth)", tt.name, tt.got, tt.want)
		}
	}
}
