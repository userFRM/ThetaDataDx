# thetadatadx-ffi

C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.

Compiled as both `cdylib` (shared library) and `staticlib` (archive). Consumed by the C++ (RAII) and TypeScript/Node.js (napi-rs) SDKs, and available to any third-party C/C++/Go/etc. consumer that wants to roll their own wrapper against the `thetadatadx_*` symbols.

> **Surface coverage:** the FFI layer exposes all three ThetaData surfaces — MDDS (historical), FPSS (streaming), and FLATFILES (whole-universe daily blobs). The flat-files entry points are `thetadatadx_flatfile_request_decoded` (pull + decode into an opaque row-list), `thetadatadx_flatfile_rows_to_arrow_ipc` (serialise to Arrow IPC bytes), `thetadatadx_flatfile_request_to_path` (raw vendor bytes straight to disk), and the matching `_rowlist_free` / `_bytes_free` cleanup helpers.

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
| `ThetaDataDxCredentials` | `thetadatadx_credentials_from_email`, `thetadatadx_credentials_from_file` | `thetadatadx_credentials_free` |
| `ThetaDataDxConfig` | `thetadatadx_config_production`, `thetadatadx_config_dev` | `thetadatadx_config_free` |
| `ThetaDataDxHistoricalClient` | `thetadatadx_historical_connect` | `thetadatadx_historical_free` |
| `ThetaDataDxClient` | `thetadatadx_client_connect` | `thetadatadx_client_free` |
| `ThetaDataDxStreamHandle` | `thetadatadx_streaming_connect` | `thetadatadx_streaming_free` |

### Historical (via ThetaDataDxHistoricalClient or ThetaDataDxClient)

Every historical endpoint is available as `thetadatadx_stock_*`, `thetadatadx_option_*`, `thetadatadx_index_*`, `thetadatadx_calendar_*`, `thetadatadx_interest_rate_*` functions. Each takes a `*const ThetaDataDxHistoricalClient` handle and returns a typed `#[repr(C)]` struct array (e.g. `ThetaDataDxEodTickArray`, `ThetaDataDxOhlcTickArray`). Callers must free with the corresponding `thetadatadx_*_array_free` function. List endpoints return `ThetaDataDxStringArray` (freed with `thetadatadx_string_array_free`).

`thetadatadx_client_historical()` returns a borrowed `*const ThetaDataDxHistoricalClient` from a unified handle - same session, no double auth.

### Streaming (via ThetaDataDxClient)

| Function | Description |
|----------|-------------|
| `thetadatadx_client_set_callback` | Register the user callback on the unified handle. The event-dispatch consumer thread invokes it for every typed FPSS event under `catch_unwind`. |
| `thetadatadx_client_subscribe` | Polymorphic subscribe — takes `ThetaDataDxSubscriptionRequest` (per-contract or full-stream) |
| `thetadatadx_client_unsubscribe` | Polymorphic unsubscribe — takes `ThetaDataDxSubscriptionRequest` |
| `thetadatadx_client_is_streaming` | Check if FPSS connection is live |
| `thetadatadx_client_active_subscriptions` | List active subscriptions (typed `ThetaDataDxSubscriptionArray`) |
| `thetadatadx_client_await_drain` | Block until the previous session's consumer has finished firing the callback (drain barrier) |
| `thetadatadx_client_reconnect` | Reconnect FPSS, drain the previous generation, and re-subscribe everything that was active |
| `thetadatadx_client_stop_streaming` | Stop streaming, historical stays alive |
| `thetadatadx_client_free` | Free the unified handle |

### Streaming (via ThetaDataDxStreamHandle, standalone)

| Function | Description |
|----------|-------------|
| `thetadatadx_streaming_connect` | Connect standalone FPSS client |
| `thetadatadx_streaming_set_callback` | Register the user callback. The event-dispatch consumer thread invokes it for every typed FPSS event under `catch_unwind`. |
| `thetadatadx_streaming_subscribe` | Polymorphic subscribe — takes `ThetaDataDxSubscriptionRequest` |
| `thetadatadx_streaming_unsubscribe` | Polymorphic unsubscribe — takes `ThetaDataDxSubscriptionRequest` |
| `thetadatadx_streaming_is_authenticated` | Check if FPSS is authenticated |
| `thetadatadx_streaming_active_subscriptions` | List active subscriptions (typed `ThetaDataDxSubscriptionArray`) |
| `thetadatadx_streaming_dropped_events` | Cumulative count of events the TLS reader could not publish into the event ring |
| `thetadatadx_streaming_await_drain` | Block until the previous session's consumer has finished firing the callback |
| `thetadatadx_streaming_reconnect` | Reconnect FPSS, drain the previous generation, and re-subscribe everything that was active |
| `thetadatadx_streaming_shutdown` | Shut down FPSS client |
| `thetadatadx_streaming_free` | Free the FPSS handle |

### Error handling

All functions that can fail return null on error. Call `thetadatadx_last_error()` to get the error message (valid until the next FFI call on the same thread).

## Memory model

- Opaque handles are heap-allocated via `Box::into_raw`, freed via `Box::from_raw` in the corresponding `*_free` function.
- Data endpoints return typed `#[repr(C)]` struct arrays (e.g. `ThetaDataDxEodTickArray { data, len }`) - free with the corresponding `thetadatadx_*_array_free` function.
- List endpoints return `ThetaDataDxStringArray` - free with `thetadatadx_string_array_free`.
- `thetadatadx_streaming_active_subscriptions` returns `*mut ThetaDataDxSubscriptionArray` - free with `thetadatadx_subscription_array_free`.
- `thetadatadx_last_error()` returns a borrowed pointer - do NOT free it.
- `thetadatadx_client_historical()` returns a borrowed pointer - do NOT free it.

## Safety

- All functions check for null handles before dereferencing.
- Mutex locks use poison recovery (`unwrap_or_else(|e| e.into_inner())`).
- `ThetaDataDxHistoricalClient` is `#[repr(transparent)]` over `HistoricalClient` for safe pointer casting.

### Panic boundary

Every `extern "C"` function (145 fns — 84 in `ffi/src/lib.rs` plus 61 generator-emitted in `ffi/src/endpoint_with_options.rs`) is wrapped in the `ffi_boundary!` macro, which `std::panic::catch_unwind(AssertUnwindSafe(|| { ... }))`s the body. Rust panics no longer cross the C ABI — the payload is downcast to `String`, routed through `tracing::error!` on target `thetadatadx::ffi::panic`, stored in the thread-local `LAST_ERROR` slot accessed by `thetadatadx_last_error()`, and the function returns the caller-declared default (`ptr::null_mut()` / `-1` / `0` / sentinel-empty-array). Before this wrapper every panic on Rust 1.81+ aborted the host process; pre-1.81 it was undefined behaviour.

Regression tests live at `ffi/tests/panic_boundary.rs`; the `thetadatadx_test_panic_{str,string}` symbols used for testing are gated behind a `testing-panic-boundary` cargo feature so the production `cdylib` never ships them.
