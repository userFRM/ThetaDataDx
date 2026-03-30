# thetadatadx-ffi

C FFI layer for `thetadatadx` - exposes the Rust SDK as `extern "C"` functions.

Compiled as both `cdylib` (shared library) and `staticlib` (archive). Consumed by the Go (CGo) and C++ (RAII) SDKs.

## Building

```bash
cargo build --release -p thetadatadx-ffi
```

Produces:
- `target/release/libthetadatadx_ffi.so` (Linux)
- `target/release/libthetadatadx_ffi.dylib` (macOS)
- `target/release/libthetadatadx_ffi.a` (static, all platforms)

## API Surface

### Handle types (opaque pointers)

| Handle | Create | Free |
|--------|--------|------|
| `TdxCredentials` | `tdx_credentials_new`, `tdx_credentials_from_file` | `tdx_credentials_free` |
| `TdxConfig` | `tdx_config_production`, `tdx_config_dev` | `tdx_config_free` |
| `TdxClient` | `tdx_client_connect` | `tdx_client_free` |
| `TdxUnified` | `tdx_unified_connect` | `tdx_unified_free` |
| `TdxFpssHandle` | `tdx_fpss_connect` | `tdx_fpss_free` |

### Historical (via TdxClient or TdxUnified)

All 61 endpoints are available as `tdx_stock_*`, `tdx_option_*`, `tdx_index_*`, `tdx_calendar_*`, `tdx_interest_rate_*` functions. Each takes a `*const TdxClient` handle and returns a `*mut c_char` (JSON string, caller must free with `tdx_string_free`).

`tdx_unified_historical()` returns a borrowed `*const TdxClient` from a unified handle - same session, no double auth.

### Streaming (via TdxUnified or TdxFpssHandle)

| Function | Description |
|----------|-------------|
| `tdx_unified_start_streaming` | Start FPSS on the unified handle |
| `tdx_unified_subscribe_quotes` | Subscribe to quote stream |
| `tdx_unified_subscribe_trades` | Subscribe to trade stream |
| `tdx_unified_next_event` | Poll for next event (JSON, blocks with timeout) |
| `tdx_unified_stop_streaming` | Stop streaming, historical stays alive |

### Error handling

All functions that can fail return null on error. Call `tdx_last_error()` to get the error message (valid until the next FFI call on the same thread).

## Memory model

- Opaque handles are heap-allocated via `Box::into_raw`, freed via `Box::from_raw` in the corresponding `*_free` function.
- String results (`*mut c_char`) are owned by the caller - free with `tdx_string_free`.
- `tdx_last_error()` returns a borrowed pointer - do NOT free it.
- `tdx_unified_historical()` returns a borrowed pointer - do NOT free it.

## Safety

- All functions check for null handles before dereferencing.
- Mutex locks use poison recovery (`unwrap_or_else(|e| e.into_inner())`).
- `TdxClient` is `#[repr(transparent)]` over `DirectClient` for safe pointer casting.
