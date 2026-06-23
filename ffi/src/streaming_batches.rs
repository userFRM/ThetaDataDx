//! C ABI for the pull-based Arrow `RecordBatch` streaming reader.
//!
//! This is the C-side surface the C++ SDK's `Stream::batches(..)` builds on.
//! It is the sibling of the per-event `thetadatadx_client_set_callback`
//! path: the same subscriptions feed it, but market-data events are pulled
//! as columnar Arrow batches under the fixed streaming schema (see the
//! core `thetadatadx::streaming::stream_batch_schema`) rather than pushed one
//! at a time.
//!
//! # Boundary format
//!
//! Each batch crosses the C ABI as an Arrow IPC stream byte buffer — the
//! same columnar exit the per-tick `thetadatadx_*_to_arrow_ipc` terminals
//! use, so a C++ caller decodes it with arrow-cpp's IPC reader exactly as it
//! already does for historical results. The fixed schema is available up
//! front as a schema-only IPC buffer so the C++ `arrow::RecordBatchReader`
//! subclass can report `schema()` before the first batch arrives.
//!
//! # Lifecycle
//!
//! `thetadatadx_client_batches_open` starts the session and returns an
//! opaque handle. `..._next_ipc` blocks for the next batch (releasing no
//! lock the caller holds); a `1` return means clean end of stream.
//! `..._close` signals shutdown through a shared reference, so it is safe to
//! call from another thread WHILE a `..._next_ipc` pull is parked: it wakes
//! the pull (which then returns end of stream) and tears the streaming session
//! down without taking exclusive ownership. `..._free` releases the handle.
//! Every entry point is wrapped in the panic boundary so no Rust panic
//! crosses `extern "C"`.
//!
//! # Concurrency and handle ownership
//!
//! The reader is held behind an `Arc` so a teardown from one thread cannot
//! deallocate the reader out from under a blocking pull parked on another
//! thread. `..._next_ipc` / `..._schema_ipc` / `..._dropped` each take a
//! short owning clone of that `Arc` for the duration of the call, so the
//! reader stays alive for the whole pull even if `..._free` runs
//! concurrently. `..._free` signals close before dropping its handle
//! reference, so a parked pull is woken (returns end of stream) and the last
//! `Arc` drop (whichever thread holds it) performs the deallocation. This
//! mirrors the Python and TypeScript readers, which hold the same core
//! [`RecordBatchStream`] behind an `Arc` and close through
//! [`RecordBatchStream::close_shared`]; a bare owned handle freed by value
//! would instead deallocate the reader while a concurrent pull still
//! borrowed it.

use std::os::raw::c_void;
use std::sync::Arc;

use thetadatadx::streaming::{Backpressure, RecordBatchStream};

use crate::error::set_error;
use crate::streaming::ThetaDataDxClient;
use crate::types::ThetaDataDxArrowBytes;

/// Backpressure policy selector for [`thetadatadx_client_batches_open`].
///
/// Mirrors `thetadatadx::streaming::Backpressure`. `BLOCK` is lossless and
/// applies backpressure to the wire; `DROP_OLDEST` keeps a bounded buffer
/// and drops the oldest batch on overflow, counted by
/// [`thetadatadx_record_batch_stream_dropped`].
pub const THETADATADX_BACKPRESSURE_BLOCK: i32 = 0;
/// Bounded-buffer, drop-oldest backpressure. See
/// [`THETADATADX_BACKPRESSURE_BLOCK`].
pub const THETADATADX_BACKPRESSURE_DROP_OLDEST: i32 = 1;

/// Opaque handle to a live pull-based Arrow `RecordBatch` reader.
///
/// Created by [`thetadatadx_client_batches_open`], drained by
/// [`thetadatadx_record_batch_stream_next_ipc`], closed by
/// [`thetadatadx_record_batch_stream_close`], freed by
/// [`thetadatadx_record_batch_stream_free`].
///
/// The reader is held behind an `Arc` so a concurrent free cannot
/// deallocate it while a blocking pull on another thread still borrows it;
/// see the module-level concurrency note.
pub struct ThetaDataDxRecordBatchStream {
    inner: Arc<RecordBatchStream>,
}

/// Open a pull-based Arrow `RecordBatch` reader over the unified client's
/// stream.
///
/// Subscriptions are managed on the same surface as the callback path
/// (`thetadatadx_client_*` subscribe entry points); subscribe first, then
/// open the reader. Starts the streaming session, so this is an alternative to
/// `thetadatadx_client_set_callback`, not a concurrent consumer.
///
/// `batch_size` rows per batch (`0` is clamped to 1). `linger_ms` is the
/// partial-batch flush deadline in milliseconds so a quiet stream still
/// delivers. `backpressure` is one of [`THETADATADX_BACKPRESSURE_BLOCK`] /
/// [`THETADATADX_BACKPRESSURE_DROP_OLDEST`]; `capacity` is the bounded-buffer
/// depth in batches for the drop-oldest mode (ignored, may be 0, for block
/// mode).
///
/// Returns a handle on success, or null with `thetadatadx_last_error()` set
/// on failure (network / auth / parse error, an unknown `backpressure`
/// value, or a stream already active on the client). Free the handle with
/// [`thetadatadx_record_batch_stream_free`].
///
/// # Safety
///
/// `handle` must be a valid pointer returned by `thetadatadx_client_connect`
/// and not yet freed, valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_client_batches_open(
    handle: *const ThetaDataDxClient,
    batch_size: usize,
    linger_ms: u64,
    backpressure: i32,
    capacity: usize,
) -> *mut ThetaDataDxRecordBatchStream {
    ffi_boundary!(std::ptr::null_mut(), {
        // SAFETY: the caller's contract guarantees `handle` is a live
        // `ThetaDataDxClient` from `thetadatadx_client_connect`, valid for
        // the call; `as_ref` yields `None` for a null pointer, handled below.
        let Some(client) = (unsafe { handle.as_ref() }) else {
            set_error("client handle is null");
            return std::ptr::null_mut();
        };
        let backpressure = match backpressure {
            THETADATADX_BACKPRESSURE_BLOCK => Backpressure::Block,
            THETADATADX_BACKPRESSURE_DROP_OLDEST => Backpressure::DropOldest {
                capacity: capacity.max(1),
            },
            other => {
                set_error(&format!(
                    "unknown backpressure value {other}; expected 0 (block) or 1 (drop-oldest)"
                ));
                return std::ptr::null_mut();
            }
        };
        let stream = client
            .inner
            .stream()
            .batches()
            .batch_size(batch_size.max(1))
            .linger(std::time::Duration::from_millis(linger_ms))
            .backpressure(backpressure)
            .build();
        match stream {
            Ok(inner) => Box::into_raw(Box::new(ThetaDataDxRecordBatchStream {
                inner: Arc::new(inner),
            })),
            Err(e) => {
                crate::error::set_error_from(&e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Block for the next batch and serialise it as an Arrow IPC stream into
/// `out`.
///
/// Returns:
/// * `0` — a batch was produced; `out` holds its IPC bytes (free with
///   [`crate::types::thetadatadx_arrow_bytes_free`]).
/// * `1` — clean end of stream; `out` is set empty (`data = null, len = 0`),
///   nothing to free.
/// * `-1` — error; `out` is set empty and `thetadatadx_last_error()` is set.
///
/// # Safety
///
/// `stream` must be a valid handle from [`thetadatadx_client_batches_open`]
/// not yet freed. `out` must be a valid, writable pointer to a
/// [`ThetaDataDxArrowBytes`]; on every return path it is fully initialised.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_record_batch_stream_next_ipc(
    stream: *const ThetaDataDxRecordBatchStream,
    out: *mut ThetaDataDxArrowBytes,
) -> i32 {
    ffi_boundary!(-1, {
        if out.is_null() {
            set_error("out pointer is null");
            return -1;
        }
        // Initialise the out-param to empty up front so every early return
        // leaves a well-formed (data=null, len=0) value the caller can read
        // and need not free.
        // SAFETY: `out` is non-null and the caller's contract guarantees it
        // is a valid writable `ThetaDataDxArrowBytes`.
        unsafe { out.write(ThetaDataDxArrowBytes::empty()) };

        // SAFETY: caller's contract guarantees `stream` is a live handle
        // from `thetadatadx_client_batches_open`, not freed; `as_ref` yields
        // `None` for a null pointer, handled below.
        let Some(stream) = (unsafe { stream.as_ref() }) else {
            set_error("stream handle is null");
            return -1;
        };
        // Take an owning clone of the reader before the blocking pull so a
        // concurrent `..._free` on another thread cannot deallocate the reader
        // while this pull is parked inside `next_blocking`. The `&stream`
        // borrow of the boxed handle ends here; the pull runs against the
        // cloned `Arc`, and a concurrent close wakes it (see the module note).
        let inner = Arc::clone(&stream.inner);
        match inner.next_blocking() {
            Ok(Some(batch)) => match crate::streaming_batches_ipc::batch_to_ipc(&batch) {
                Ok(bytes) => {
                    // SAFETY: `out` validated non-null + writable above.
                    unsafe { out.write(ThetaDataDxArrowBytes::from_vec(bytes)) };
                    0
                }
                Err(msg) => {
                    set_error(&msg);
                    -1
                }
            },
            Ok(None) => 1,
            Err(e) => {
                crate::error::set_error_from(&thetadatadx::Error::from(e));
                -1
            }
        }
    })
}

/// Serialise the reader's fixed schema as a schema-only Arrow IPC stream
/// into `out`, so a C++ `arrow::RecordBatchReader` can report `schema()`
/// before any batch arrives.
///
/// Returns `0` on success (`out` holds the schema IPC bytes, free with
/// [`crate::types::thetadatadx_arrow_bytes_free`]) or `-1` on error (`out`
/// set empty, `thetadatadx_last_error()` set).
///
/// # Safety
///
/// Same handle / `out` contract as
/// [`thetadatadx_record_batch_stream_next_ipc`].
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_record_batch_stream_schema_ipc(
    stream: *const ThetaDataDxRecordBatchStream,
    out: *mut ThetaDataDxArrowBytes,
) -> i32 {
    ffi_boundary!(-1, {
        if out.is_null() {
            set_error("out pointer is null");
            return -1;
        }
        // SAFETY: `out` non-null; caller guarantees it is writable.
        unsafe { out.write(ThetaDataDxArrowBytes::empty()) };
        // SAFETY: caller's contract guarantees `stream` is a live handle
        // from `thetadatadx_client_batches_open`, not freed; `as_ref` yields
        // `None` for a null pointer, handled below.
        let Some(stream) = (unsafe { stream.as_ref() }) else {
            set_error("stream handle is null");
            return -1;
        };
        // Own the reader for the call so a concurrent `..._free` cannot
        // deallocate it mid-read (see the module concurrency note).
        let inner = Arc::clone(&stream.inner);
        match crate::streaming_batches_ipc::schema_to_ipc(&inner.schema()) {
            Ok(bytes) => {
                // SAFETY: `out` validated writable above.
                unsafe { out.write(ThetaDataDxArrowBytes::from_vec(bytes)) };
                0
            }
            Err(msg) => {
                set_error(&msg);
                -1
            }
        }
    })
}

/// Number of batches dropped so far under the drop-oldest backpressure
/// policy. Always `0` under the block policy.
///
/// # Safety
///
/// `stream` must be a valid handle from [`thetadatadx_client_batches_open`]
/// not yet freed.
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_record_batch_stream_dropped(
    stream: *const ThetaDataDxRecordBatchStream,
) -> u64 {
    ffi_boundary!(0, {
        // SAFETY: caller's contract guarantees `stream` is a live handle
        // from `thetadatadx_client_batches_open`, not freed; `as_ref` yields
        // `None` for a null pointer, handled below.
        match unsafe { stream.as_ref() } {
            // Own the reader for the read so a concurrent `..._free` cannot
            // deallocate it mid-call (see the module concurrency note).
            Some(stream) => Arc::clone(&stream.inner).dropped(),
            None => {
                set_error("stream handle is null");
                0
            }
        }
    })
}

/// Stop the reader: unsubscribe and tear the streaming session down, WITHOUT
/// freeing the handle.
///
/// Signals shutdown through a shared reference, so it is safe to call from a
/// different thread while a [`thetadatadx_record_batch_stream_next_ipc`] pull
/// is parked: it wakes the pull (which returns `1`, clean end of stream) and
/// shuts the session down. Idempotent; safe to call any number of times. The
/// handle remains valid and must still be released with
/// [`thetadatadx_record_batch_stream_free`].
///
/// This is the teardown a multi-threaded caller (e.g. a control thread that
/// stops a reader another thread is draining) should use: it never takes
/// exclusive ownership of the handle, so it cannot race the in-flight pull
/// the way freeing the handle by value would.
///
/// # Safety
///
/// `stream` must be a valid handle from [`thetadatadx_client_batches_open`]
/// not yet freed, or null (a null is a no-op).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_record_batch_stream_close(
    stream: *const ThetaDataDxRecordBatchStream,
) {
    ffi_boundary!((), {
        // SAFETY: caller's contract guarantees `stream` is a live handle from
        // `_open`, not freed; `as_ref` yields `None` for null, a no-op.
        if let Some(stream) = unsafe { stream.as_ref() } {
            // Shared-reference close: wakes any in-flight pull on another
            // thread and tears the session down without exclusive ownership.
            stream.inner.close_shared();
        }
    })
}

/// Release the reader handle.
///
/// Signals shutdown first (waking any in-flight pull, which then returns
/// clean end of stream) and then drops this handle's reference to the
/// reader. Because the reader is held behind an `Arc` and every pull takes
/// its own clone for the duration of the call, the underlying reader is
/// deallocated only once the last reference drops, so a
/// [`thetadatadx_record_batch_stream_next_ipc`] pull parked on another thread
/// at the moment of free is woken and completes against still-live memory
/// rather than being deallocated out from under it. After this call the
/// handle is invalid and must not be used.
///
/// # Safety
///
/// `stream` must be a valid handle from [`thetadatadx_client_batches_open`]
/// not yet freed, or null (a null is a no-op).
#[no_mangle]
pub unsafe extern "C" fn thetadatadx_record_batch_stream_free(
    stream: *mut ThetaDataDxRecordBatchStream,
) {
    ffi_boundary!((), {
        if stream.is_null() {
            return;
        }
        // SAFETY: caller's contract guarantees `stream` is a live handle
        // from `_open`, not previously freed.
        let boxed = unsafe { Box::from_raw(stream) };
        // Signal close before dropping this handle's `Arc`, so a pull parked
        // on another thread is woken and returns end of stream. Dropping
        // `boxed` then releases this reference; the reader itself is
        // deallocated by whichever thread drops the last `Arc` (its core
        // `Drop` re-signals close idempotently), never while a concurrent
        // pull still holds a clone.
        boxed.inner.close_shared();
        drop(boxed);
    })
}

// The opaque handle carries a raw owner pointer across the boundary; the
// pointer is only ever produced and consumed by these functions, never
// dereferenced by foreign code.
const _: () = {
    // Compile-time reminder that the handle is pointer-sized opaque state.
    assert!(
        std::mem::size_of::<*mut ThetaDataDxRecordBatchStream>()
            == std::mem::size_of::<*mut c_void>()
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Every reader entry point treats a null handle as a well-defined no-op
    /// (or error return), never a deref of null. The out-param functions
    /// additionally leave `out` initialised empty so a caller can always read
    /// it. The live-handle pull / close / free concurrency contract (that a
    /// teardown never deallocates the reader out from under an in-flight pull)
    /// is held by the `Arc` ownership here and proven in the core
    /// `fpss::batch_reader` tests (`close_from_another_handle_unblocks_a_parked_pull`);
    /// it needs a live streaming connection, so it is exercised there rather than
    /// reconstructed against a mock in this layer.
    #[test]
    fn null_handle_is_a_safe_no_op_on_every_entry_point() {
        // close / free / dropped on null: no deref, no panic.
        // SAFETY: passing null is explicitly part of each function's contract.
        unsafe {
            thetadatadx_record_batch_stream_close(std::ptr::null());
            thetadatadx_record_batch_stream_free(std::ptr::null_mut());
            assert_eq!(thetadatadx_record_batch_stream_dropped(std::ptr::null()), 0);
        }

        // next_ipc / schema_ipc on a null handle: -1, and `out` left as the
        // empty sentinel so the caller can always read it and has nothing to
        // free. The out-param is seeded empty (the documented pre-call state);
        // the callee overwrites with the empty sentinel and owns no buffer.
        let mut next_out = ThetaDataDxArrowBytes::empty();
        let mut schema_out = ThetaDataDxArrowBytes::empty();
        // SAFETY: null handle + valid writable out-param; contract returns -1
        // and writes an empty `out`.
        let rc_next =
            unsafe { thetadatadx_record_batch_stream_next_ipc(std::ptr::null(), &mut next_out) };
        // SAFETY: null handle + valid writable out-param; contract returns -1
        // and writes an empty `out`.
        let rc_schema = unsafe {
            thetadatadx_record_batch_stream_schema_ipc(std::ptr::null(), &mut schema_out)
        };
        assert_eq!(rc_next, -1);
        assert_eq!(rc_schema, -1);
        assert!(next_out.data.is_null() && next_out.len == 0);
        assert!(schema_out.data.is_null() && schema_out.len == 0);

        // A null `out` is rejected without a deref.
        // SAFETY: null handle + null out; contract returns -1.
        let rc_null_out = unsafe {
            thetadatadx_record_batch_stream_next_ipc(std::ptr::null(), std::ptr::null_mut())
        };
        assert_eq!(rc_null_out, -1);
    }
}
