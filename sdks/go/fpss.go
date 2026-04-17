package thetadatadx

/*
#include "ffi_bridge.h"
*/
import "C"

import (
	"fmt"
	"runtime"
)


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

// FpssControlKind identifies the sub-type of a control event.
// Use with FpssControlData.Kind.
type FpssControlKind = int32

const (
	FpssCtrlLoginSuccess      FpssControlKind = 0
	FpssCtrlContractAssigned  FpssControlKind = 1
	FpssCtrlReqResponse       FpssControlKind = 2
	FpssCtrlMarketOpen        FpssControlKind = 3
	FpssCtrlMarketClose       FpssControlKind = 4
	FpssCtrlServerError       FpssControlKind = 5
	FpssCtrlDisconnected      FpssControlKind = 6
	FpssCtrlReconnecting      FpssControlKind = 8
	FpssCtrlReconnected       FpssControlKind = 9
	FpssCtrlError             FpssControlKind = 10
	FpssCtrlUnknownFrame      FpssControlKind = 11 // ID = frame code, Detail = hex payload
)

// FpssQuote is a real-time quote event from FPSS.
// Bid and Ask are pre-decoded to float64 at parse time.
type FpssQuote struct {
	ContractID   int32
	MsOfDay      int32
	BidSize      int32
	BidExchange  int32
	Bid          float64
	BidCondition int32
	AskSize      int32
	AskExchange  int32
	Ask          float64
	AskCondition int32
	Date         int32
	ReceivedAtNs uint64
}

// FpssTrade is a real-time trade event from FPSS.
// Price is pre-decoded to float64 at parse time.
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
// Open/High/Low/Close are pre-decoded to float64 at parse time.
type FpssOhlcvc struct {
	ContractID   int32
	MsOfDay      int32
	Open         float64
	High         float64
	Low          float64
	Close        float64
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
//
// Pins the goroutine to one OS thread across the cgo call + TLS error
// read because the FFI's last-error slot is a Rust thread_local.
// See `docs/dev/w3-async-cancellation-design.md` "cgo thread-local
// correctness".
func NewFpssClient(creds *Credentials, config *Config) (*FpssClient, error) {
	if creds == nil || creds.handle == nil {
		return nil, fmt.Errorf("thetadatadx: credentials handle is nil")
	}
	if config == nil || config.handle == nil {
		return nil, fmt.Errorf("thetadatadx: config handle is nil")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	h := C.tdx_fpss_connect(creds.handle, config.handle)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &FpssClient{handle: h}, nil
}

// fpssCall is called by the generated subscribe/unsubscribe wrappers in
// fpss_methods.go. Every such wrapper has already pinned its goroutine
// via runtime.LockOSThread() so the in-flight cgo call and this
// lastError() read both run on the same OS thread.
func (f *FpssClient) fpssCall(rc C.int) (int, error) {
	if rc < 0 {
		return int(rc), fmt.Errorf("thetadatadx: %s", lastError())
	}
	return int(rc), nil
}

// Subscription represents a single active subscription entry.
type Subscription struct {
	Kind     string // "Quote", "Trade", or "OpenInterest"
	Contract string // "SPY" or "SPY 20260417 550 C"
}

// Close frees the FPSS handle. Call after Shutdown.
func (f *FpssClient) Close() {
	if f.handle != nil {
		C.tdx_fpss_free(f.handle)
		f.handle = nil
	}
}
