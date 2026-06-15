//! Options-aware historical endpoint wrappers emitted by the generator.
//!
//! The generated `endpoint_request_options.rs` declares `TdxEndpointRequestOptions`
//! and the private helper `apply_endpoint_request_options`. The generated
//! `endpoint_with_options.rs` declares the 61 `tdx_<endpoint>_with_options`
//! entry points. The generated `endpoint_stream.rs` declares the
//! `tdx_<endpoint>_stream` server-stream entry points. All three are
//! `include!`'d here so the shared helper + callback ABI are in scope when the
//! generated wrappers expand.

use std::os::raw::c_char;
use std::os::raw::c_void;
use std::ptr;

use crate::error::{set_error, set_error_from};
use crate::runtime;
use crate::types::{
    insert_bool_arg, insert_float_arg, insert_int_arg, insert_optional_str_arg,
    TdxCalendarDayArray, TdxEodTickArray, TdxGreeksAllTickArray, TdxGreeksEodTickArray,
    TdxGreeksFirstOrderTickArray, TdxGreeksSecondOrderTickArray, TdxGreeksThirdOrderTickArray,
    TdxHistoricalClient, TdxIndexPriceAtTimeTickArray, TdxInterestRateTickArray, TdxIvTickArray,
    TdxMarketValueTickArray, TdxOhlcTickArray, TdxOpenInterestTickArray, TdxOptionContractArray,
    TdxPriceTickArray, TdxQuoteTickArray, TdxStringArray, TdxTradeGreeksAllTickArray,
    TdxTradeGreeksFirstOrderTickArray, TdxTradeGreeksImpliedVolatilityTickArray,
    TdxTradeGreeksSecondOrderTickArray, TdxTradeGreeksThirdOrderTickArray, TdxTradeQuoteTickArray,
    TdxTradeTickArray,
};

// ── Historical server-stream callback C ABI ──

/// User callback signature for the `tdx_<endpoint>_stream` entry points:
/// invoked once per decoded gRPC chunk drained from a historical result.
///
/// `rows` points at the first element of a contiguous run of `len` tick
/// structs — the SAME `#[repr(C)]` layout the matching
/// `tdx_<endpoint>_with_options` array exposes (e.g. a
/// `tdx_option_history_trade_stream` chunk is `len` × `TdxTradeTick`). Cast
/// `rows` to the endpoint's tick pointer type before indexing. The pointer is
/// valid only for the duration of the call; copy any rows the caller wants to
/// outlive the callback. A null `rows` with `len == 0` is never emitted — an
/// empty result drains as zero callback invocations.
///
/// `ctx` is the opaque user context registered alongside the callback; it is
/// passed back unchanged on every invocation and never dereferenced by Rust.
pub type TdxTickChunkCallback = extern "C" fn(rows: *const c_void, len: usize, ctx: *mut c_void);

/// Bundle of `(callback, ctx)` carried into the per-chunk closure the core
/// `request.stream` primitive takes by value. The core wraps the handler in a
/// `Mutex` and may invoke it from a runtime worker thread, so the bundle is
/// `Send`; the contained `*mut c_void` is the user's opaque payload, never
/// dereferenced by Rust.
#[derive(Clone, Copy)]
struct TickChunkSink {
    callback: TdxTickChunkCallback,
    ctx: *mut c_void,
}

// SAFETY: `ctx` is the user's opaque context — never dereferenced by Rust,
// only handed back to the user's `extern "C" fn` exactly as registered.
// Send-across-threads safety of whatever `ctx` points at is the caller's
// documented responsibility (same contract as `TdxStreamCallback`).
unsafe impl Send for TickChunkSink {}

impl TickChunkSink {
    /// Forward one chunk to the registered C callback. `rows` / `len` come
    /// straight from the decoder-owned slice (`chunk.as_ptr()` /
    /// `chunk.len()`), so there is no copy or re-marshaling on this path.
    fn emit(&self, rows: *const c_void, len: usize) {
        (self.callback)(rows, len, self.ctx);
    }
}

include!("endpoint_request_options.rs");
include!("endpoint_with_options.rs");
include!("endpoint_stream.rs");
