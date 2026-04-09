# Macro Guide for Contributors

This guide explains the macro system used by the generated `DirectClient`
surface.

> [!IMPORTANT]
> Most contributors should **not** add endpoints by hand with macro invocations.
> The current endpoint workflow is:
> - update `crates/thetadatadx/proto/external.proto` when the wire contract changes
> - update `crates/thetadatadx/endpoint_surface.toml` for the normalized SDK surface
> - update `crates/thetadatadx/endpoint_schema.toml` if a new `DataTable` layout is introduced
>
> The build then generates the registry, shared endpoint runtime, and
> `DirectClient` declarations automatically. This guide is for understanding and
> maintaining the macro layer that those generators target.

## `parsed_endpoint!` -- the core macro

Every non-streaming `DirectClient` endpoint ultimately expands through
`parsed_endpoint!` in `crates/thetadatadx/src/direct.rs`. A single invocation
generates three things:

1. **A builder struct** (e.g., `StockHistoryOhlcBuilder`) that holds required
   params as owned fields and optional params as `Option<T>` fields.
2. **Chainable setter methods** on the builder for each optional parameter.
3. **An `IntoFuture` impl** so that `.await`-ing the builder executes the gRPC
   call, collects the response stream, and parses the `DataTable` into typed ticks.

The `DirectClient` gets a method that constructs and returns the builder.

### Invocation anatomy

```rust
parsed_endpoint! {
    /// Doc comment shown on the client method.
    builder StockHistoryOhlcBuilder;
    fn stock_history_ohlc(
        symbol: str,         // required params with type tags
        date: str,
        interval: str
    ) -> Vec<OhlcTick>;      // return type
    grpc: get_stock_history_ohlc;     // gRPC stub method name
    request: StockHistoryOhlcRequest; // protobuf request wrapper type
    query: StockHistoryOhlcParams {   // protobuf params struct + field mapping
        root: symbol.to_string(),
        date: date.to_string(),
        ivl: interval.to_string(),
    };
    parse: decode::parse_ohlc_ticks;  // DataTable -> Vec<T> parser
    dates: date;                      // optional: validate YYYYMMDD format
    optional {                        // optional params (chainable setters)
        venue: opt_str = None,
        start_time: opt_str = None,
        end_time: opt_str = None,
    }
}
```

### What gets generated

```rust
// 1. Builder struct
pub struct StockHistoryOhlcBuilder<'a> {
    client: &'a DirectClient,
    pub(crate) symbol: String,       // required (str -> String)
    pub(crate) date: String,
    pub(crate) interval: String,
    pub(crate) venue: Option<String>, // optional (opt_str -> Option<String>)
    pub(crate) start_time: Option<String>,
    pub(crate) end_time: Option<String>,
}

// 2. Setters
impl<'a> StockHistoryOhlcBuilder<'a> {
    pub fn venue(mut self, v: &str) -> Self { ... }
    pub fn start_time(mut self, v: &str) -> Self { ... }
    pub fn end_time(mut self, v: &str) -> Self { ... }
}

// 3. IntoFuture -- makes `.await` work
impl<'a> IntoFuture for StockHistoryOhlcBuilder<'a> { ... }

// 4. Client method
impl DirectClient {
    pub fn stock_history_ohlc(&self, symbol: &str, date: &str, interval: &str)
        -> StockHistoryOhlcBuilder<'_> { ... }
}
```

Usage:

```rust
// Simple
let ticks = client.stock_history_ohlc("AAPL", "20260401", "1m").await?;

// With options
let ticks = client.stock_history_ohlc("AAPL", "20260401", "1m")
    .venue("arca")
    .start_time("04:00:00")
    .await?;
```

## Type tag system

Required and optional parameters use short "type tags" instead of raw Rust types.
The helper macros (`req_field_type!`, `req_param_type!`, `opt_field_type!`,
`opt_setter!`) expand each tag into the correct types.

### Required parameter tags

| Tag       | Struct field type | Constructor param type | Notes                    |
|-----------|-------------------|----------------------|--------------------------|
| `str`     | `String`          | `&str`               | Most common              |
| `str_vec` | `Vec<String>`     | `&[&str]`            | Multi-symbol endpoints   |

### Optional parameter tags

| Tag        | Field type        | Setter param type | Notes                         |
|------------|-------------------|-------------------|-------------------------------|
| `opt_str`  | `Option<String>`  | `&str`            | String options (venue, time)  |
| `opt_i32`  | `Option<i32>`     | `i32`             | Integer options (limit)       |
| `opt_f64`  | `Option<f64>`     | `f64`             | Float options                 |
| `opt_bool` | `Option<bool>`    | `bool`            | Boolean flags                 |
| `string`   | `String`          | `&str`            | Required-with-default string  |

## Adding a new endpoint (current workflow)

The supported path for new endpoints is spec-driven, not hand-written macro
expansion.

### 1. Define the tick type (if new)

Add a `[types.YourTick]` block in `crates/thetadatadx/endpoint_schema.toml`.
`build.rs` generates the `parse_your_ticks()` function automatically. See
`docs/endpoint-schema.md` for the TOML format. The tick structs themselves live
in `crates/tdbe/`. Set `contract_id = true` if the tick type should carry
contract identification fields (`expiration`/`strike`/`right`).

### 2. Add the protobuf types (if new message)

Update `crates/thetadatadx/proto/external.proto` to add the request/params
messages. `cargo build` regenerates Rust types.

### 3. Add the endpoint surface entry

Update `crates/thetadatadx/endpoint_surface.toml` with the normalized endpoint
name, REST path, return kind, and parameter semantics. Reuse existing
`param_groups` and `templates` where possible instead of copying large parameter
blocks.

### 4. Build and inspect the generated surfaces

Run `cargo build`. The generator validates `endpoint_surface.toml` against
`external.proto` and emits the registry, shared endpoint runtime, and
`DirectClient` endpoint declarations.

You only need to edit the macro layer or `build_support/endpoints.rs` if the
new endpoint cannot be expressed by the existing surface specification model.

### 5. Expose in SDKs

- **FFI**: add `extern "C"` wrapper in `ffi/src/lib.rs` (see FFI macros below)
- **Python**: add PyO3 method in `sdks/python/src/lib.rs`
- **Go**: add method in `sdks/go/client.go`
- **C++**: add in `sdks/cpp/include/thetadx.hpp` and `sdks/cpp/src/thetadx.cpp`

### 6. Update CHANGELOG.md

Add to `[Unreleased]`.

## `streaming_endpoint!` -- chunked streaming

`streaming_endpoint!` is nearly identical to `parsed_endpoint!` but instead of
`IntoFuture` (which collects the entire response), it generates a `.stream(handler)`
method that calls the handler with each chunk of ticks as they arrive.

Key difference: the builder has no `IntoFuture` impl. Instead:

```rust
impl<'a> StockHistoryTradeStreamBuilder<'a> {
    pub async fn stream<F>(self, mut handler: F) -> Result<(), Error>
    where
        F: FnMut(&[TradeTick]),
    { ... }
}
```

Usage:

```rust
client.stock_history_trade_stream("AAPL", "20260401")
    .start_time("04:00:00")
    .stream(|ticks| {
        println!("got {} ticks", ticks.len());
    })
    .await?;
```

## Helper macros for endpoint families

Groups of endpoints that share the same required/optional parameter signatures
are wrapped in family-specific macros to reduce duplication.

### `option_snapshot_greeks_endpoint!`

Wraps `parsed_endpoint!` for the 5 option-snapshot-greeks variants. All take
`(symbol, expiration, strike, right)` as required params plus the same set of
optional params.

### `option_history_greeks_interval_endpoint!`

For interval-based option history greeks endpoints. Adds `date` and `interval`
to the required params.

### `option_history_trade_greeks_endpoint!`

For trade-level option history greeks (no interval). Takes `date` as an
additional required param.

These macros call `parsed_endpoint!` internally -- they only factor out
the repeated parameter lists.

## FFI macros (`ffi/src/lib.rs`)

The FFI layer uses its own macro set to wrap the Rust builders into
`#[no_mangle] extern "C"` functions.

| Macro                          | Purpose                                               |
|--------------------------------|-------------------------------------------------------|
| `tick_array_type!`             | Defines a `#[repr(C)]` array struct with `from_vec()` and `free()` |
| `tick_array_free!`             | Generates the `extern "C"` free function for a tick array |
| `ffi_typed_endpoint!`          | Wraps a typed endpoint with C string params            |
| `ffi_typed_endpoint_no_params!`| Wraps a typed endpoint with no params                  |
| `ffi_typed_snapshot_endpoint!` | Wraps a snapshot endpoint (takes C string array of symbols)|
| `ffi_list_endpoint!`           | Wraps a list endpoint with C string params             |
| `ffi_list_endpoint_no_params!` | Wraps a list endpoint with no params                   |

The pattern for adding an FFI endpoint:

```rust
// 1. Define the array type (if new tick type)
tick_array_type!(TdxVwapTickArray, VwapTick);
tick_array_free!(tdx_vwap_tick_array_free, TdxVwapTickArray);

// 2. Wrap the endpoint
ffi_typed_endpoint!(
    /// Fetch historical VWAP data.
    tdx_stock_history_vwap => stock_history_vwap, TdxVwapTickArray(symbol, start, end)
);
```
