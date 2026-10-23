//! Options-aware market-data endpoint wrappers emitted by the generator.
//!
//! The generated `endpoint_request_options.rs` declares `ThetaDataDxEndpointRequestOptions`
//! and the private helper `apply_endpoint_request_options`. The generated
//! `endpoint_with_options.rs` declares one
//! `thetadatadx_<endpoint>_with_options` entry point per buffered endpoint. The generated `endpoint_stream.rs` declares the
//! `thetadatadx_<endpoint>_stream` server-stream entry points. All three are
//! `include!`'d here so the shared helper + callback ABI are in scope when the
//! generated wrappers expand.

use std::os::raw::c_char;
use std::os::raw::c_void;

use crate::error::{set_error, set_error_from};
use crate::runtime;
use crate::types::{
    ThetaDataDxCalendarDayArray, ThetaDataDxColumnPresence, ThetaDataDxEodTickArray,
    ThetaDataDxGreeksAllTickArray, ThetaDataDxGreeksEodTickArray,
    ThetaDataDxGreeksFirstOrderTickArray, ThetaDataDxGreeksSecondOrderTickArray,
    ThetaDataDxGreeksThirdOrderTickArray, ThetaDataDxIndexPriceAtTimeTickArray,
    ThetaDataDxInterestRateTickArray, ThetaDataDxIntervalQuoteTickArray, ThetaDataDxIvTickArray,
    ThetaDataDxMarketDataClient, ThetaDataDxMarketValueTickArray, ThetaDataDxOhlcTickArray,
    ThetaDataDxOpenInterestTickArray, ThetaDataDxOptionContractArray, ThetaDataDxPriceTickArray,
    ThetaDataDxQuoteTickArray, ThetaDataDxStringArray, ThetaDataDxTradeGreeksAllTickArray,
    ThetaDataDxTradeGreeksFirstOrderTickArray, ThetaDataDxTradeGreeksImpliedVolatilityTickArray,
    ThetaDataDxTradeGreeksSecondOrderTickArray, ThetaDataDxTradeGreeksThirdOrderTickArray,
    ThetaDataDxTradeQuoteTickArray, ThetaDataDxTradeTickArray, insert_bool_arg, insert_float_arg,
    insert_int_arg, insert_optional_str_arg,
};

// ── Market-data server-stream callback C ABI ──

/// User callback signature for the `thetadatadx_<endpoint>_stream` entry points:
/// invoked once per decoded gRPC chunk drained from a market-data result.
///
/// `rows` points at the first element of a contiguous run of `len` tick
/// structs, the SAME `#[repr(C)]` layout the matching
/// `thetadatadx_<endpoint>_with_options` array exposes (e.g. a
/// `thetadatadx_option_history_trade_stream` chunk is `len` × `ThetaDataDxTradeTick`). Cast
/// `rows` to the endpoint's tick pointer type before indexing. The pointer is
/// valid only for the duration of the call; copy any rows the caller wants to
/// outlive the callback. A null `rows` with `len == 0` is never emitted: an
/// empty result drains as zero callback invocations.
///
/// `columns` is the chunk's column presence, never null: the columns the
/// chunk's wire carried and the root each row belongs to. When the chunk's
/// rows span several roots, as in a flat file, `symbols` holds one root per
/// row (`symbols_len == len`); when every row shares one, as in option and
/// index history, `symbol` holds it. A response that carries no root, such
/// as stock history, sets neither. The library owns it: it is valid only for
/// the duration of the call and must not be freed by the callback.
///
/// `ctx` is the opaque user context registered alongside the callback; it is
/// passed back unchanged on every invocation and never dereferenced by Rust.
///
/// The callback runs under the C ABI and must not unwind across the boundary.
/// A C++ `throw` or a C `longjmp` that escapes the callback into the calling
/// Rust frame is undefined behavior. The stream path wraps each invocation in
/// [`std::panic::catch_unwind`], but that contains only a Rust panic raised on
/// our side of the boundary, not a foreign exception out of the callback.
/// Catch and handle every exception inside the callback before returning.
pub type ThetaDataDxTickChunkCallback = extern "C" fn(
    rows: *const c_void,
    len: usize,
    columns: *const ThetaDataDxColumnPresence,
    ctx: *mut c_void,
);

/// Bundle of `(callback, ctx)` carried into the per-chunk closure the core
/// `request.stream` primitive takes by value. The core wraps the handler in a
/// `Mutex` and may invoke it from a runtime worker thread, so the bundle is
/// `Send`; the contained `*mut c_void` is the user's opaque payload, never
/// dereferenced by Rust.
#[derive(Clone, Copy)]
struct TickChunkSink {
    callback: ThetaDataDxTickChunkCallback,
    ctx: *mut c_void,
}

// SAFETY: `ctx` is the user's opaque context — never dereferenced by Rust,
// only handed back to the user's `extern "C" fn` exactly as registered.
// Send-across-threads safety of whatever `ctx` points at is the caller's
// documented responsibility (same contract as `ThetaDataDxStreamCallback`).
unsafe impl Send for TickChunkSink {}

impl TickChunkSink {
    /// Forward one chunk to the registered C callback. `rows` / `len` come
    /// straight from the decoder-owned slice (`chunk.as_ptr()` /
    /// `chunk.len()`), so there is no copy or re-marshaling of the rows.
    /// `columns` crosses as a C carrier built for this call and released
    /// once the callback returns, which is why the callback must not keep or
    /// free it. Empty chunks are skipped so the public contract remains
    /// "empty result drains as zero callback invocations" and no caller ever
    /// receives a non-null dangling pointer with `len == 0`.
    ///
    /// The core wraps this handler in a `Mutex` and may drive it from a
    /// runtime worker thread (see the type doc), not on the
    /// `ffi_boundary!`-guarded entry point. A Rust panic raised on this path
    /// would otherwise unwind across the C ABI on a foreign thread, so wrap
    /// the invocation in `catch_unwind`, the same defence the reconnect
    /// callback and the stream dispatcher apply. This contains a panic from
    /// our own Rust code; it does not contain a foreign exception thrown out
    /// of the user callback. The callback's no-unwind contract (an exception
    /// or `longjmp` escaping it is undefined behavior) is documented on
    /// `ThetaDataDxTickChunkCallback`.
    fn emit(
        &self,
        rows: *const c_void,
        len: usize,
        columns: &thetadatadx::columns::ColumnPresence,
    ) {
        if len == 0 {
            return;
        }
        let columns = ThetaDataDxColumnPresence::from_presence(columns);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.callback)(rows, len, &columns, self.ctx);
        }));
        // SAFETY: `columns` was built by `from_presence` above, the callback
        // only borrowed it, and this is its one release.
        unsafe { crate::types::thetadatadx_column_presence_free(columns) };
        if outcome.is_err() {
            tracing::error!(
                target: "thetadatadx::ffi",
                "tick-chunk callback panicked; chunk dropped and the panic contained at the C boundary",
            );
        }
    }
}

/// Run one buffered endpoint to completion on the shared runtime.
///
/// Every generated `thetadatadx_<endpoint>*` entry calls this rather than
/// wrapping `invoke_endpoint` in an `async` block of its own. Each such block
/// is a distinct future type embedding the dispatch future, which spans every
/// endpoint, so `block_on` and the future's drop glue were compiled once per
/// endpoint: work that grew with the square of the endpoint count. One
/// non-generic call site keeps it to a single instantiation.
#[inline(never)]
fn invoke_endpoint_blocking(
    client: &thetadatadx::mdds::MarketDataClient,
    name: &str,
    args: &thetadatadx::EndpointArgs,
) -> Result<thetadatadx::EndpointOutput, thetadatadx::EndpointError> {
    runtime().block_on(thetadatadx::endpoint::invoke_endpoint(
        client, name, args, None,
    ))
}

/// Streaming counterpart of [`invoke_endpoint_blocking`]. The chunk handler
/// is built here, once, so `invoke_endpoint_stream`, which is generic over
/// it, is instantiated for one closure type instead of one per endpoint.
#[inline(never)]
fn invoke_endpoint_stream_blocking(
    client: &thetadatadx::mdds::MarketDataClient,
    name: &str,
    args: &thetadatadx::EndpointArgs,
    sink: TickChunkSink,
) -> Result<(), thetadatadx::EndpointError> {
    runtime().block_on(thetadatadx::endpoint::invoke_endpoint_stream(
        client,
        name,
        args,
        move |rows, len, columns| sink.emit(rows, len, columns),
    ))
}

include!("endpoint_request_options.rs");
include!("endpoint_with_options.rs");
include!("endpoint_stream.rs");

// A C caller can pass a null chunk callback. It is representable only as the
// `None` niche of `Option<ThetaDataDxTickChunkCallback>`, which every
// `thetadatadx_<endpoint>_stream` entry rejects before building its sink.
// This coercion stops compiling if the generated parameter reverts to a
// non-nullable `extern "C" fn`.
const _: unsafe extern "C" fn(
    *const ThetaDataDxMarketDataClient,
    *const c_char,
    *const c_char,
    *const c_char,
    Option<ThetaDataDxTickChunkCallback>,
    *mut c_void,
    *const ThetaDataDxEndpointRequestOptions,
) -> i32 = thetadatadx_stock_history_eod_stream;

#[cfg(test)]
mod tests {
    use super::*;
    use thetadatadx::columns::ColumnPresence;

    /// Chunk callback that records the root of each row it was handed, read
    /// the way a C caller reads it: `symbols[i]` when set, else `symbol`.
    extern "C" fn record_roots(
        _rows: *const c_void,
        len: usize,
        columns: *const ThetaDataDxColumnPresence,
        ctx: *mut c_void,
    ) {
        let text = |p: *const c_char| {
            (!p.is_null()).then(|| {
                // SAFETY: every pointer the carrier holds is null or a
                // NUL-terminated C string the sink keeps alive for the call.
                unsafe { std::ffi::CStr::from_ptr(p) }
                    .to_str()
                    .unwrap()
                    .to_owned()
            })
        };
        // SAFETY: the sink hands a carrier that lives for the call, and `ctx`
        // is the test's `Vec`, borrowed for the call.
        let (columns, seen) = unsafe { (&*columns, &mut *ctx.cast::<Vec<Vec<Option<String>>>>()) };
        seen.push(if columns.symbols.is_null() {
            vec![text(columns.symbol); len]
        } else {
            // SAFETY: a non-null `symbols` points at `symbols_len` C strings.
            unsafe { std::slice::from_raw_parts(columns.symbols, columns.symbols_len) }
                .iter()
                .map(|&p| text(p))
                .collect()
        });
    }

    /// A C caller streaming a flat file attributes each row from the presence
    /// its chunk callback receives, whether the chunk's rows span several
    /// roots or share one.
    #[test]
    fn chunk_callback_receives_each_rows_root() {
        let mut seen: Vec<Vec<Option<String>>> = Vec::new();
        let sink = TickChunkSink {
            callback: record_roots,
            ctx: std::ptr::from_mut(&mut seen).cast(),
        };
        let rows = [0_u8; 2];
        let per_row = ColumnPresence::from_names(["bid"]).with_symbols(["AAPL", "MSFT"]);
        sink.emit(rows.as_ptr().cast(), 2, &per_row);
        let shared = ColumnPresence::from_names(["bid"]).with_symbol("SPY");
        sink.emit(rows.as_ptr().cast(), 2, &shared);

        let root = |s: &str| Some(s.to_owned());
        assert_eq!(
            seen,
            [
                vec![root("AAPL"), root("MSFT")],
                vec![root("SPY"), root("SPY")]
            ]
        );
    }
}
