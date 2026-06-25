<p align="center">
  <img src="../assets/logo.svg" alt="ThetaDataDx" width="500" />
</p>

# thetadatadx-ffi

C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.

Compiled as both `cdylib` (shared library) and `staticlib` (archive). Consumed by the C++ (RAII) and TypeScript/Node.js (napi-rs) SDKs, and available to any third-party C/C++ consumer that wants to roll their own wrapper against the `thetadatadx_*` symbols.

> **Surface coverage:** the FFI layer exposes all three ThetaData surfaces — historical, streaming, and FLATFILES (whole-universe daily blobs). The flat-files entry points are `thetadatadx_flatfile_request_decoded` (pull + decode into an opaque row-list), `thetadatadx_flatfile_rows_to_arrow_ipc` (serialise to Arrow IPC bytes), `thetadatadx_flatfile_request_to_path` (raw vendor bytes straight to disk), and the matching `_rowlist_free` / `_bytes_free` cleanup helpers.

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
| `ThetaDataDxCredentials` | `thetadatadx_credentials_from_email`, `thetadatadx_credentials_from_file`, `thetadatadx_credentials_from_api_key`, `thetadatadx_credentials_from_api_key_with_email`, `thetadatadx_credentials_from_env_or_file`, `thetadatadx_credentials_from_dotenv` | `thetadatadx_credentials_free` |
| `ThetaDataDxConfig` | `thetadatadx_config_production`, `thetadatadx_config_dev` | `thetadatadx_config_free` |
| `ThetaDataDxHistoricalClient` | `thetadatadx_historical_connect` | `thetadatadx_historical_free` |
| `ThetaDataDxClient` | `thetadatadx_client_connect` | `thetadatadx_client_free` |
| `ThetaDataDxStreamHandle` | `thetadatadx_streaming_connect` | `thetadatadx_streaming_free` |

Credentials accept either an API key or an email/password pair. `thetadatadx_credentials_from_api_key` (and `thetadatadx_credentials_from_api_key_with_email`) take a key the caller generated from the ThetaData user portal; `thetadatadx_credentials_from_env_or_file` reads the key from the `THETADATA_API_KEY` environment variable when it is set and non-empty, otherwise falls back to the two-line creds file at the given path. `thetadatadx_credentials_from_dotenv` reads the same `THETADATA_API_KEY` (or a `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair) from a `.env`-format file.

### Historical (via ThetaDataDxHistoricalClient or ThetaDataDxClient)

Every historical endpoint is available as `thetadatadx_stock_*`, `thetadatadx_option_*`, `thetadatadx_index_*`, `thetadatadx_calendar_*`, `thetadatadx_interest_rate_*` functions. Each takes a `*const ThetaDataDxHistoricalClient` handle and returns a typed `#[repr(C)]` struct array (e.g. `ThetaDataDxEodTickArray`, `ThetaDataDxOhlcTickArray`). Callers must free with the corresponding `thetadatadx_*_array_free` function. List endpoints return `ThetaDataDxStringArray` (freed with `thetadatadx_string_array_free`).

`thetadatadx_client_historical()` returns a borrowed `*const ThetaDataDxHistoricalClient` from a unified handle - same session, no double auth.

### Streaming (via ThetaDataDxClient)

| Function | Description |
|----------|-------------|
| `thetadatadx_client_set_callback` | Register the user callback on the unified handle. The streaming delivery thread invokes it for every typed streaming event. Any Rust panic on the dispatch path is contained at the boundary; the callback itself must not unwind across the C ABI (see Panic boundary). |
| `thetadatadx_client_subscribe` | Polymorphic subscribe — takes `ThetaDataDxSubscriptionRequest` (per-contract or full-stream) |
| `thetadatadx_client_unsubscribe` | Polymorphic unsubscribe — takes `ThetaDataDxSubscriptionRequest` |
| `thetadatadx_client_is_streaming` | Check if the streaming connection is live |
| `thetadatadx_client_active_subscriptions` | List active subscriptions (typed `ThetaDataDxSubscriptionArray`) |
| `thetadatadx_client_await_drain` | Block until the previous session's consumer has finished firing the callback (drain barrier) |
| `thetadatadx_client_reconnect` | Reconnect streaming, drain the previous generation, and re-subscribe everything that was active |
| `thetadatadx_client_stop_streaming` | Stop streaming, historical stays alive |
| `thetadatadx_client_free` | Free the unified handle |

### Streaming (via ThetaDataDxStreamHandle, standalone)

| Function | Description |
|----------|-------------|
| `thetadatadx_streaming_connect` | Connect standalone streaming client |
| `thetadatadx_streaming_set_callback` | Register the user callback. The streaming delivery thread invokes it for every typed streaming event. Any Rust panic on the dispatch path is contained at the boundary; the callback itself must not unwind across the C ABI (see Panic boundary). |
| `thetadatadx_streaming_subscribe` | Polymorphic subscribe — takes `ThetaDataDxSubscriptionRequest` |
| `thetadatadx_streaming_unsubscribe` | Polymorphic unsubscribe — takes `ThetaDataDxSubscriptionRequest` |
| `thetadatadx_streaming_is_authenticated` | Check if the streaming client is authenticated |
| `thetadatadx_streaming_active_subscriptions` | List active subscriptions (typed `ThetaDataDxSubscriptionArray`) |
| `thetadatadx_streaming_dropped_events` | Cumulative count of events the TLS reader could not publish into the event ring |
| `thetadatadx_streaming_await_drain` | Block until the previous session's consumer has finished firing the callback |
| `thetadatadx_streaming_reconnect` | Reconnect streaming, drain the previous generation, and re-subscribe everything that was active |
| `thetadatadx_streaming_shutdown` | Shut down the streaming client |
| `thetadatadx_streaming_free` | Free the streaming handle |

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

Every `extern "C"` function (all 145 of them) contains panics at the boundary. A Rust panic no longer crosses the C ABI: the panic message is recorded and retrievable via the C error API (`thetadatadx_last_error()`), and the function returns the caller-declared default (`ptr::null_mut()` / `-1` / `0` / sentinel-empty-array). Before this, every panic on Rust 1.81+ aborted the host process; pre-1.81 it was undefined behaviour.

This boundary contains panics raised by our own Rust code. It does not contain a foreign exception that unwinds into a Rust frame from a caller-supplied callback. A C++ `throw` or a C `longjmp` that escapes a registered callback (a streaming event callback, a tick-chunk callback, or a reconnect-decision callback) across the C ABI is undefined behavior, the same as for any C library, and the boundary cannot intercept it. The caller's contract is that a callback must not unwind across the boundary: catch and handle every exception inside the callback before it returns. The C++ wrapper's `set_callback` shim already does this for you (it is `noexcept` and swallows any exception its `std::function` raises). Each callback type documents this no-unwind contract.

The boundary is covered by regression tests; the test-only panic symbols are gated behind a cargo feature so the production library never ships them.
