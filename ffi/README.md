# thetadatadx-ffi

C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.

Compiled as both `cdylib` (shared library) and `staticlib` (archive). Consumed by the C++ (RAII) and TypeScript/Node.js (napi-rs) SDKs, and available to any third-party C/C++/Go/etc. consumer that wants to roll their own wrapper against the `tdx_*` symbols.

> **Surface coverage:** the FFI layer exposes all three ThetaData surfaces — MDDS (historical), FPSS (streaming), and FLATFILES (whole-universe daily blobs). The flat-files entry points are `tdx_flatfile_request_decoded` (pull + decode into an opaque row-list), `tdx_flatfile_rows_to_arrow_ipc` (serialise to Arrow IPC bytes), `tdx_flatfile_request_to_path` (raw vendor bytes straight to disk), and the matching `_rowlist_free` / `_bytes_free` cleanup helpers.

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
| `TdxCredentials` | `tdx_credentials_from_email`, `tdx_credentials_from_file` | `tdx_credentials_free` |
| `TdxConfig` | `tdx_config_production`, `tdx_config_dev` | `tdx_config_free` |
| `TdxHistoricalClient` | `tdx_historical_connect` | `tdx_historical_free` |
| `TdxClient` | `tdx_client_connect` | `tdx_client_free` |
| `TdxStreamHandle` | `tdx_streaming_connect` | `tdx_streaming_free` |

### Historical (via TdxHistoricalClient or TdxClient)

Every historical endpoint is available as `tdx_stock_*`, `tdx_option_*`, `tdx_index_*`, `tdx_calendar_*`, `tdx_interest_rate_*` functions. Each takes a `*const TdxHistoricalClient` handle and returns a typed `#[repr(C)]` struct array (e.g. `TdxEodTickArray`, `TdxOhlcTickArray`). Callers must free with the corresponding `tdx_*_array_free` function. List endpoints return `TdxStringArray` (freed with `tdx_string_array_free`).

`tdx_client_historical()` returns a borrowed `*const TdxHistoricalClient` from a unified handle - same session, no double auth.

### Streaming (via TdxClient)

| Function | Description |
|----------|-------------|
| `tdx_client_set_callback` | Register the user callback on the unified handle. The event-dispatch consumer thread invokes it for every typed FPSS event under `catch_unwind`. |
| `tdx_client_subscribe` | Polymorphic subscribe — takes `TdxSubscriptionRequest` (per-contract or full-stream) |
| `tdx_client_unsubscribe` | Polymorphic unsubscribe — takes `TdxSubscriptionRequest` |
| `tdx_client_is_streaming` | Check if FPSS connection is live |
| `tdx_client_active_subscriptions` | List active subscriptions (typed `TdxSubscriptionArray`) |
| `tdx_client_await_drain` | Block until the previous session's consumer has finished firing the callback (drain barrier) |
| `tdx_client_reconnect` | Reconnect FPSS, drain the previous generation, and re-subscribe everything that was active |
| `tdx_client_stop_streaming` | Stop streaming, historical stays alive |
| `tdx_client_free` | Free the unified handle |

### Streaming (via TdxStreamHandle, standalone)

| Function | Description |
|----------|-------------|
| `tdx_streaming_connect` | Connect standalone FPSS client |
| `tdx_streaming_set_callback` | Register the user callback. The event-dispatch consumer thread invokes it for every typed FPSS event under `catch_unwind`. |
| `tdx_streaming_subscribe` | Polymorphic subscribe — takes `TdxSubscriptionRequest` |
| `tdx_streaming_unsubscribe` | Polymorphic unsubscribe — takes `TdxSubscriptionRequest` |
| `tdx_streaming_is_authenticated` | Check if FPSS is authenticated |
| `tdx_streaming_active_subscriptions` | List active subscriptions (typed `TdxSubscriptionArray`) |
| `tdx_streaming_dropped_events` | Cumulative count of events the TLS reader could not publish into the event ring |
| `tdx_streaming_await_drain` | Block until the previous session's consumer has finished firing the callback |
| `tdx_streaming_reconnect` | Reconnect FPSS, drain the previous generation, and re-subscribe everything that was active |
| `tdx_streaming_shutdown` | Shut down FPSS client |
| `tdx_streaming_free` | Free the FPSS handle |

### Error handling

All functions that can fail return null on error. Call `tdx_last_error()` to get the error message (valid until the next FFI call on the same thread).

## Memory model

- Opaque handles are heap-allocated via `Box::into_raw`, freed via `Box::from_raw` in the corresponding `*_free` function.
- Data endpoints return typed `#[repr(C)]` struct arrays (e.g. `TdxEodTickArray { data, len }`) - free with the corresponding `tdx_*_array_free` function.
- List endpoints return `TdxStringArray` - free with `tdx_string_array_free`.
- `tdx_streaming_active_subscriptions` returns `*mut TdxSubscriptionArray` - free with `tdx_subscription_array_free`.
- `tdx_last_error()` returns a borrowed pointer - do NOT free it.
- `tdx_client_historical()` returns a borrowed pointer - do NOT free it.

## Safety

- All functions check for null handles before dereferencing.
- Mutex locks use poison recovery (`unwrap_or_else(|e| e.into_inner())`).
- `TdxHistoricalClient` is `#[repr(transparent)]` over `HistoricalClient` for safe pointer casting.

### Panic boundary

Every `extern "C"` function (145 fns — 84 in `ffi/src/lib.rs` plus 61 generator-emitted in `ffi/src/endpoint_with_options.rs`) is wrapped in the `ffi_boundary!` macro, which `std::panic::catch_unwind(AssertUnwindSafe(|| { ... }))`s the body. Rust panics no longer cross the C ABI — the payload is downcast to `String`, routed through `tracing::error!` on target `thetadatadx::ffi::panic`, stored in the thread-local `LAST_ERROR` slot accessed by `tdx_last_error()`, and the function returns the caller-declared default (`ptr::null_mut()` / `-1` / `0` / sentinel-empty-array). Before this wrapper every panic on Rust 1.81+ aborted the host process; pre-1.81 it was undefined behaviour.

Regression tests live at `ffi/tests/panic_boundary.rs`; the `tdx_test_panic_{str,string}` symbols used for testing are gated behind a `testing-panic-boundary` cargo feature so the production `cdylib` never ships them.
