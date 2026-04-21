package thetadatadx

/*
#include "ffi_bridge.h"
*/
import "C"

import (
	"fmt"
	"runtime"
)

// ── Client ──
// Lifecycle: intentionally hand-written (language-specific constructor semantics).

// Client holds a connection to the ThetaData MDDS server.
type Client struct {
	handle *C.TdxClient
}

/*
// EndpointRequestOptions and EndpointOption helpers are generated in
// endpoint_options.go.
*/

// Connect authenticates and connects to ThetaData.
//
// Pins the goroutine to one OS thread across the cgo call + TLS error
// read because the FFI's last-error slot is a Rust thread_local; without
// the pin, Go's runtime could migrate the goroutine and the error read
// would see an empty slot on the wrong thread.
func Connect(creds *Credentials, config *Config) (*Client, error) {
	if creds == nil || creds.handle == nil {
		return nil, fmt.Errorf("thetadatadx: credentials handle is nil")
	}
	if config == nil || config.handle == nil {
		return nil, fmt.Errorf("thetadatadx: config handle is nil")
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	h := C.tdx_client_connect(creds.handle, config.handle)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &Client{handle: h}, nil
}

// Close frees the client handle and disconnects.
func (c *Client) Close() {
	if c.handle != nil {
		C.tdx_client_free(c.handle)
		c.handle = nil
	}
}

/*
// endpointRequestOptionsToC is generated in endpoint_options.go.
// Offline utilities are generated in utilities.go.
// Public tick types are generated in tick_structs.go.
// C-mirror FFI structs live in tick_ffi_mirrors.go.
// Array converters are generated in tick_converters.go.
*/
