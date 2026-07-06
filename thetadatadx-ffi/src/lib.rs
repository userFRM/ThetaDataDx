// Reason: FFI extern "C" functions use raw pointers, pattern matching, and C-style
// conventions that are fundamentally incompatible with many pedantic lints (let-else
// on nullable pointers, doc_markdown on C identifiers, ptr_cast_constness on FFI
// boundary types). Fixing these would make the FFI code less idiomatic for C interop.
#![allow(clippy::pedantic)]
#![warn(missing_docs)]

//! C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.
//!
//! This crate is compiled as both `cdylib` (shared library) and `staticlib` (archive).
//! It is consumed by the C++ SDK and is the supported integration path for any
//! third-party C/C++ consumer that wants to roll their own wrapper.
//!
//! # Safety
//!
//! All `unsafe extern "C"` functions in this crate follow the same safety contract:
//!
//! - Pointer arguments must be either null (handled gracefully) or valid pointers
//!   obtained from a prior `thetadatadx_*` call.
//! - `*const c_char` arguments must point to valid, NUL-terminated C strings.
//! - Returned typed arrays are heap-allocated and must be freed with the
//!   corresponding `thetadatadx_*_free` function.
//! - Functions are not thread-safe on the same handle; callers must synchronize.
//!
//! # Memory model
//!
//! - Opaque handles (`*mut ThetaDataDxMarketDataClient`, `*mut ThetaDataDxCredentials`, etc.) are heap-allocated
//!   via `Box::into_raw` and freed via the corresponding `thetadatadx_*_free` function.
//! - Tick arrays are returned as `#[repr(C)]` structs with a `data` pointer and `len`.
//!   They MUST be freed with the corresponding `thetadatadx_*_array_free` function.
//! - String arrays (`ThetaDataDxStringArray`) must be freed with `thetadatadx_string_array_free`.
//! - The caller MUST free every non-null pointer / non-empty array returned by this library.
//!
//! # Error handling
//!
//! Functions that can fail return an empty array (data=null, len=0) on error and set
//! a thread-local error string retrievable via `thetadatadx_last_error`.

// Reason: every `unsafe extern "C"` in this crate shares one safety
// contract, documented in the crate-level docstring above (pointer
// arguments null-or-valid, C-string NUL-terminated, caller frees every
// non-null return). Per-fn `# Safety` sections would duplicate that
// paragraph 145 times and drift from the centralized version.
#![allow(clippy::missing_safety_doc)]

use std::sync::OnceLock;

// ── Global tokio runtime (same pattern as the Python bindings) ──

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Build (or return the already-built) process-global async runtime,
/// sizing the worker pool from the first client's
/// [`thetadatadx::RuntimeConfig`].
///
/// The embedded runtime is process-global and built exactly once: the
/// first connect in the process seeds it from that client's
/// `config.runtime`, so `thetadatadx_config_set_worker_threads` takes effect for
/// the first client created in the process. Later connects share the
/// already-built pool and their `runtime` config is a no-op by design.
pub(crate) fn runtime_from_config(
    cfg: &thetadatadx::RuntimeConfig,
) -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        cfg.build_runtime()
            .expect("failed to create tokio runtime for thetadatadx-ffi")
    })
}

/// Build the process-global runtime from `cfg` and report its worker
/// count. Test-only hook proving `thetadatadx_config_set_worker_threads` reaches
/// the tokio builder; not part of the C ABI.
#[doc(hidden)]
pub fn __test_runtime_worker_count(cfg: &thetadatadx::RuntimeConfig) -> usize {
    runtime_from_config(cfg).metrics().num_workers()
}

/// Return the process-global async runtime, building it with tokio
/// default sizing if no client has seeded it yet.
///
/// Connect functions seed the pool from config via
/// [`runtime_from_config`]; every post-connect endpoint call resolves the
/// already-built runtime through this accessor.
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        thetadatadx::RuntimeConfig::default()
            .build_runtime()
            .expect("failed to create tokio runtime for thetadatadx-ffi")
    })
}

// ── rustls provider install (no module-init hook on a C ABI) ──

/// Seat the ring `CryptoProvider` as the process-wide rustls default before
/// the first TLS handshake.
///
/// The Python and TypeScript bindings install it from their module-init
/// hooks and the CLI / server binaries from `main`; a C ABI library has no
/// equivalent load-time entrypoint, so every connect function seats the
/// provider here instead. Guarded by `Once` so the cost is paid once and
/// concurrent first-connects from multiple threads serialise cleanly.
pub(crate) fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = thetadatadx::__internal_install_ring_crypto_provider();
    });
}

// ── Module layout ──
//
// Macros must be declared before the modules that use them, hence `#[macro_use]`
// on `panic` (which exports `ffi_boundary!`) and `error` (which exports
// `require_cstr!`). Every other module pulls its macro callers via crate-root
// hoisting.
//
// All `#[no_mangle] extern "C" fn` symbols remain identical regardless of the
// Rust module they live in — the module split is purely organizational.

#[macro_use]
mod panic;

#[macro_use]
mod error;

pub mod auth;
pub mod endpoints;
pub mod flatfiles;
pub mod streaming;
pub mod streaming_batches;
mod streaming_batches_ipc;
pub mod types;
pub mod utility;

// ── Public re-exports ──
//
// Downstream Rust users (notably the feature-gated integration test at
// `thetadatadx-ffi/tests/panic_boundary.rs`) expect to resolve the FFI functions via the
// crate root. Re-export everything so `use thetadatadx_ffi::thetadatadx_last_error`
// keeps working after the split.

pub use crate::auth::*;
pub use crate::endpoints::*;
pub use crate::error::*;
pub use crate::flatfiles::*;
pub use crate::streaming::*;
pub use crate::streaming_batches::*;
pub use crate::types::*;
pub use crate::utility::*;
