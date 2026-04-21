// Reason: FFI extern "C" functions use raw pointers, pattern matching, and C-style
// conventions that are fundamentally incompatible with many pedantic lints (let-else
// on nullable pointers, doc_markdown on C identifiers, ptr_cast_constness on FFI
// boundary types). Fixing these would make the FFI code less idiomatic for C interop.
#![allow(clippy::pedantic)]

//! C FFI layer for `thetadatadx` — exposes the Rust SDK as `extern "C"` functions.
//!
//! This crate is compiled as both `cdylib` (shared library) and `staticlib` (archive).
//! It is consumed by the Go (`CGo`) and C++ SDKs.
//!
//! # Safety
//!
//! All `unsafe extern "C"` functions in this crate follow the same safety contract:
//!
//! - Pointer arguments must be either null (handled gracefully) or valid pointers
//!   obtained from a prior `tdx_*` call.
//! - `*const c_char` arguments must point to valid, NUL-terminated C strings.
//! - Returned typed arrays are heap-allocated and must be freed with the
//!   corresponding `tdx_*_free` function.
//! - Functions are not thread-safe on the same handle; callers must synchronize.
//!
//! # Memory model
//!
//! - Opaque handles (`*mut TdxClient`, `*mut TdxCredentials`, etc.) are heap-allocated
//!   via `Box::into_raw` and freed via the corresponding `tdx_*_free` function.
//! - Tick arrays are returned as `#[repr(C)]` structs with a `data` pointer and `len`.
//!   They MUST be freed with the corresponding `tdx_*_array_free` function.
//! - String arrays (`TdxStringArray`) must be freed with `tdx_string_array_free`.
//! - The caller MUST free every non-null pointer / non-empty array returned by this library.
//!
//! # Error handling
//!
//! Functions that can fail return an empty array (data=null, len=0) on error and set
//! a thread-local error string retrievable via `tdx_last_error`.

// Reason: every `unsafe extern "C"` in this crate shares one safety
// contract, documented in the crate-level docstring above (pointer
// arguments null-or-valid, C-string NUL-terminated, caller frees every
// non-null return). Per-fn `# Safety` sections would duplicate that
// paragraph 145 times and drift from the centralized version.
#![allow(clippy::missing_safety_doc)]

use std::sync::OnceLock;

// ── Global tokio runtime (same pattern as the Python bindings) ──

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to create tokio runtime for thetadatadx-ffi")
    })
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
pub mod historical;
pub mod streaming;
pub mod types;
pub mod utility;

// ── Public re-exports ──
//
// Downstream Rust users (notably the feature-gated integration test at
// `ffi/tests/panic_boundary.rs`) expect to resolve the FFI functions via the
// crate root. Re-export everything so `use thetadatadx_ffi::tdx_last_error`
// keeps working after the split.

pub use crate::auth::*;
pub use crate::error::*;
pub use crate::historical::*;
pub use crate::streaming::*;
pub use crate::types::*;
pub use crate::utility::*;
