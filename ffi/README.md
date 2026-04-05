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

All 61 endpoints are available as `tdx_stock_*`, `tdx_option_*`, `tdx_index_*`, `tdx_calendar_*`, `tdx_interest_rate_*` functions. Each takes a `*const TdxClient` handle and returns a typed `#[repr(C)]` struct array (e.g. `TdxEodTickArray`, `TdxOhlcTickArray`). Callers must free with the corresponding `tdx_*_array_free` function. List endpoints return `TdxStringArray` (freed with `tdx_string_array_free`).

`tdx_unified_historical()` returns a borrowed `*const TdxClient` from a unified handle - same session, no double auth.

### Streaming (via TdxUnified)

| Function | Description |
|----------|-------------|
| `tdx_unified_start_streaming` | Start FPSS on the unified handle |
| `tdx_unified_subscribe_quotes` | Subscribe to quote stream |
| `tdx_unified_subscribe_trades` | Subscribe to trade stream |
| `tdx_unified_subscribe_open_interest` | Subscribe to open interest stream |
| `tdx_unified_subscribe_full_trades` | Subscribe to firehose trade stream |
| `tdx_unified_subscribe_full_open_interest` | Subscribe to firehose open interest stream |
| `tdx_unified_unsubscribe_quotes` | Unsubscribe from quote stream |
| `tdx_unified_unsubscribe_trades` | Unsubscribe from trade stream |
| `tdx_unified_unsubscribe_open_interest` | Unsubscribe from open interest stream |
| `tdx_unified_unsubscribe_full_trades` | Unsubscribe from firehose trade stream |
| `tdx_unified_unsubscribe_full_open_interest` | Unsubscribe from firehose open interest stream |
| `tdx_unified_is_streaming` | Check if FPSS connection is live |
| `tdx_unified_contract_lookup` | Look up contract by ID |
| `tdx_unified_active_subscriptions` | List active subscriptions (typed `TdxSubscriptionArray`) |
| `tdx_unified_next_event` | Poll for next event (`*mut TdxFpssEvent`, blocks with timeout) |
| `tdx_unified_stop_streaming` | Stop streaming, historical stays alive |
| `tdx_unified_free` | Free the unified handle |

### Streaming (via TdxFpssHandle, standalone)

| Function | Description |
|----------|-------------|
| `tdx_fpss_connect` | Connect standalone FPSS client |
| `tdx_fpss_subscribe_quotes` | Subscribe to quote stream |
| `tdx_fpss_subscribe_trades` | Subscribe to trade stream |
| `tdx_fpss_subscribe_open_interest` | Subscribe to open interest stream |
| `tdx_fpss_subscribe_full_trades` | Subscribe to firehose trade stream |
| `tdx_fpss_subscribe_full_open_interest` | Subscribe to firehose open interest stream |
| `tdx_fpss_unsubscribe_quotes` | Unsubscribe from quote stream |
| `tdx_fpss_unsubscribe_trades` | Unsubscribe from trade stream |
| `tdx_fpss_unsubscribe_open_interest` | Unsubscribe from open interest stream |
| `tdx_fpss_unsubscribe_full_trades` | Unsubscribe from firehose trade stream |
| `tdx_fpss_unsubscribe_full_open_interest` | Unsubscribe from firehose open interest stream |
| `tdx_fpss_is_authenticated` | Check if FPSS is authenticated |
| `tdx_fpss_contract_lookup` | Look up contract by ID |
| `tdx_fpss_active_subscriptions` | List active subscriptions (typed `TdxSubscriptionArray`) |
| `tdx_fpss_next_event` | Poll for next event (`*mut TdxFpssEvent`, blocks with timeout) |
| `tdx_fpss_shutdown` | Shut down FPSS client |
| `tdx_fpss_free` | Free the FPSS handle |

### Error handling

All functions that can fail return null on error. Call `tdx_last_error()` to get the error message (valid until the next FFI call on the same thread).

## Memory model

- Opaque handles are heap-allocated via `Box::into_raw`, freed via `Box::from_raw` in the corresponding `*_free` function.
- Data endpoints return typed `#[repr(C)]` struct arrays (e.g. `TdxEodTickArray { data, len }`) - free with the corresponding `tdx_*_array_free` function.
- List endpoints return `TdxStringArray` - free with `tdx_string_array_free`.
- `tdx_fpss_next_event` / `tdx_unified_next_event` return `*mut TdxFpssEvent` (tagged `#[repr(C)]` struct) - free with `tdx_fpss_event_free`.
- `tdx_fpss_active_subscriptions` returns `*mut TdxSubscriptionArray` - free with `tdx_subscription_array_free`.
- `tdx_last_error()` returns a borrowed pointer - do NOT free it.
- `tdx_unified_historical()` returns a borrowed pointer - do NOT free it.

## Safety

- All functions check for null handles before dereferencing.
- Mutex locks use poison recovery (`unwrap_or_else(|e| e.into_inner())`).
- `TdxClient` is `#[repr(transparent)]` over `DirectClient` for safe pointer casting.
