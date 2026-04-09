package thetadatadx

/*
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -lthetadatadx_ffi -lm -ldl -lpthread
#cgo darwin LDFLAGS: -framework Security -framework SystemConfiguration
#include "ffi_bridge.h"
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// lastError returns the most recent FFI error string.
func lastError() string {
	p := C.tdx_last_error()
	if p == nil {
		return "unknown error"
	}
	return C.GoString(p)
}

// stringArrayToGo converts a TdxStringArray to a Go []string and frees the C memory.
func stringArrayToGo(arr C.TdxStringArray) ([]string, error) {
	if arr.data == nil || arr.len == 0 {
		C.tdx_string_array_free(arr)
		return nil, nil
	}
	n := int(arr.len)
	// Create a Go slice backed by the C array of char* pointers.
	ptrs := unsafe.Slice((**C.char)(arr.data), n)
	result := make([]string, n)
	for i := 0; i < n; i++ {
		if ptrs[i] != nil {
			result[i] = C.GoString(ptrs[i])
		}
	}
	C.tdx_string_array_free(arr)
	return result, nil
}

// ── Credentials ──

// Credentials holds ThetaData authentication credentials.
type Credentials struct {
	handle *C.TdxCredentials
}

// NewCredentials creates credentials from email and password.
func NewCredentials(email, password string) (*Credentials, error) {
	cEmail := C.CString(email)
	cPassword := C.CString(password)
	defer C.free(unsafe.Pointer(cEmail))
	defer C.free(unsafe.Pointer(cPassword))

	h := C.tdx_credentials_new(cEmail, cPassword)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &Credentials{handle: h}, nil
}

// CredentialsFromFile loads credentials from a file (line 1 = email, line 2 = password).
func CredentialsFromFile(path string) (*Credentials, error) {
	cPath := C.CString(path)
	defer C.free(unsafe.Pointer(cPath))

	h := C.tdx_credentials_from_file(cPath)
	if h == nil {
		return nil, fmt.Errorf("thetadatadx: %s", lastError())
	}
	return &Credentials{handle: h}, nil
}

// Close frees the credentials handle.
func (c *Credentials) Close() {
	if c.handle != nil {
		C.tdx_credentials_free(c.handle)
		c.handle = nil
	}
}

// ── Config ──

// Config holds connection configuration.
type Config struct {
	handle *C.TdxConfig
}

// ProductionConfig returns the production server config (ThetaData NJ datacenter).
func ProductionConfig() *Config {
	return &Config{handle: C.tdx_config_production()}
}

// DevConfig returns the dev FPSS config (port 20200, infinite historical replay).
func DevConfig() *Config {
	return &Config{handle: C.tdx_config_dev()}
}

// StageConfig returns the stage FPSS config (port 20100, testing, unstable).
func StageConfig() *Config {
	return &Config{handle: C.tdx_config_stage()}
}

// SetReconnectPolicy sets the FPSS auto-reconnect policy.
//   - 0 = Auto (default): auto-reconnect matching Java terminal behavior.
//   - 1 = Manual: no auto-reconnect, user calls reconnect explicitly.
func (c *Config) SetReconnectPolicy(policy int) {
	C.tdx_config_set_reconnect_policy(c.handle, C.int(policy))
}

// SetFlushMode sets the FPSS write flush mode.
//   - 0 = Batched (default): flush only on PING every 100ms.
//   - 1 = Immediate: flush after every frame write.
func (c *Config) SetFlushMode(mode int) {
	C.tdx_config_set_flush_mode(c.handle, C.int(mode))
}

// SetDeriveOhlcvc sets whether to derive OHLCVC bars locally from trade events.
//   - true (default): derive OHLCVC bars from trades.
//   - false: only emit server-sent OHLCVC frames (lower overhead).
func (c *Config) SetDeriveOhlcvc(enabled bool) {
	v := 0
	if enabled {
		v = 1
	}
	C.tdx_config_set_derive_ohlcvc(c.handle, C.int(v))
}

// Close frees the config handle.
func (c *Config) Close() {
	if c.handle != nil {
		C.tdx_config_free(c.handle)
		c.handle = nil
	}
}

// symbolsToCArray converts a Go string slice into a C array of C strings.
// The caller must free each element and the array itself with C.free.
func symbolsToCArray(symbols []string) (**C.char, C.size_t) {
	n := len(symbols)
	if n == 0 {
		return nil, 0
	}
	// Allocate an array of *C.char pointers.
	cArray := C.malloc(C.size_t(n) * C.size_t(unsafe.Sizeof((*C.char)(nil))))
	if cArray == nil {
		panic("thetadatadx: C.malloc returned nil")
	}
	ptrs := unsafe.Slice((**C.char)(cArray), n)
	for i, s := range symbols {
		ptrs[i] = C.CString(s)
	}
	return (**C.char)(cArray), C.size_t(n)
}

// freeSymbolArray frees a C array of C strings allocated by symbolsToCArray.
func freeSymbolArray(arr **C.char, n C.size_t) {
	if arr == nil {
		return
	}
	ptrs := unsafe.Slice(arr, int(n))
	for i := range ptrs {
		C.free(unsafe.Pointer(ptrs[i]))
	}
	C.free(unsafe.Pointer(arr))
}
