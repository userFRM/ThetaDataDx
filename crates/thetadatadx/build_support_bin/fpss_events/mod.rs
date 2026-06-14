//! FPSS streaming event schema → per-language typed struct generator.
//!
//! Reads `fpss_event_schema.toml` and emits:
//!
//! - `sdks/python/src/_generated/fpss_event_classes.rs` — `#[pyclass]` per Data variant
//!   plus the `fpss_event_to_typed` dispatcher (borrowed `&FpssEvent` →
//!   pyclass, no intermediate) plus `register_fpss_event_classes`.
//! - `sdks/typescript/src/_generated/fpss_event_classes.rs` — `#[napi(object)]` per
//!   Data variant + a flat `FpssEvent` wrapper struct with optional typed
//!   payload fields + a `buffered_event_to_typed` dispatcher that converts
//!   an internal `BufferedEvent` into one. Control/simple/raw-data events
//!   fall through to string-tagged variants so TypeScript consumers can
//!   discriminate without losing the diagnostic payload.
//!
//! Same pattern the tick generator uses for MDDS (`build_support/ticks/*`).
//! The TS target deliberately produces a flat discriminated struct (one
//! `kind` tag plus optional per-variant payloads) rather than a serde-tagged
//! napi enum: napi-rs 3.x's enum-over-object support is still rough around
//! union lowering, while `#[napi(object)]` structs with `Option<T>` payloads
//! lower to a clean TypeScript union with full IntelliSense.
//!
//! Layout:
//! * [`schema`] — TOML-backed data types + `Schema` loader.
//! * [`common`] — shared helpers (case conversion, option detection,
//!   per-language Rust type mapping).
//! * [`buffered`] — `BufferedEvent` enum + `fpss_event_to_buffered`
//!   converter emitted into the TypeScript SDK crate (the napi dispatcher
//!   consumes the buffered form; the Python dispatcher converts directly).
//! * [`python`] / [`typescript`] — per-SDK typed event classes + dispatcher.
//! * [`ffi_rust`] — `#[repr(C)]` structs + converter for the Rust FFI crate.
//! * [`ffi_c`] — C mirror header `#include`'d from the C++ SDK.

use std::path::Path;

use super::ticks::GeneratedSourceFile;

mod buffered;
mod common;
mod cpp_asserts;
mod ffi_c;
mod ffi_rust;
mod layout;
mod python;
mod schema;
mod typescript;

/// Renders the per-language FPSS event sources and writes each to its path under the repository root.
pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let abs = repo_root.join(file.relative_path);
        std::fs::write(&abs, &file.contents)?;
    }
    Ok(())
}

/// Verifies that the checked-in FPSS event sources match freshly rendered output, returning an error on drift.
pub fn check_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let abs = repo_root.join(file.relative_path);
        let current = std::fs::read_to_string(&abs).unwrap_or_default();
        if current.replace("\r\n", "\n") != file.contents {
            return Err(format!(
                "generated file out of date: {} (run: cargo run -p thetadatadx --bin generate_sdk_surfaces --features config-file)",
                file.relative_path
            )
            .into());
        }
    }
    Ok(())
}

fn render_sdk_generated_files() -> Result<Vec<GeneratedSourceFile>, Box<dyn std::error::Error>> {
    let schema = schema::load_schema()?;
    // Single shared `BufferedEvent` definition + `fpss_event_to_buffered`
    // converter — identical for Python and TypeScript. Emitting the same
    // content to two files rather than factoring into a shared crate so
    // each SDK stays a single `include!` away from the source of truth
    // with no extra crate dependency. Field types are native Rust
    // primitives (String, i32, u64, Vec<u8>, Option<...>); the FFI
    // conversion happens in the per-language `buffered_event_to_typed`
    // dispatcher which knows the napi/pyclass surface.
    let buffered = buffered::render_buffered_event_file(&schema);
    Ok(vec![
        // TypeScript keeps the shared `BufferedEvent` intermediate: its
        // typed dispatcher takes owned values out of an mpsc-crossed
        // buffered form. The Python SDK converts the borrowed
        // `&FpssEvent` directly to the pyclass (see `python.rs`), so it
        // carries no `BufferedEvent` copy.
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/_generated/buffered_event.rs",
            contents: buffered,
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/_generated/fpss_event_classes.rs",
            contents: python::render_python_fpss_event_classes(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/_generated/fpss_event_classes.rs",
            contents: typescript::render_ts_fpss_event_classes(&schema),
        },
        // Rust FFI `#[repr(C)]` structs + ZERO consts. Included from
        // `ffi/src/lib.rs` before `FfiBufferedEvent` so the hand-written
        // backing-memory wrapper can name the generated event struct.
        GeneratedSourceFile {
            relative_path: "ffi/src/fpss_event_structs.rs",
            contents: ffi_rust::render_ffi_fpss_event_structs(&schema),
        },
        // Rust FFI `fpss_event_to_ffi` converter. Included from
        // `ffi/src/lib.rs` after `FfiBufferedEvent` so it can name the
        // hand-written backing-memory wrapper.
        GeneratedSourceFile {
            relative_path: "ffi/src/fpss_event_converter.rs",
            contents: ffi_rust::render_ffi_fpss_event_converter(&schema),
        },
        // C header mirror of the FFI event structs. `#include`'d from
        // `sdks/cpp/include/thetadx.h` for the C++ SDK. Keeping the
        // schema as SSOT means C++ can never drift from Rust in field
        // order again.
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/fpss_event_structs.h.inc",
            contents: ffi_c::render_c_fpss_event_header(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/fpss_layout_asserts.hpp.inc",
            contents: cpp_asserts::render_cpp_fpss_layout_asserts(&schema),
        },
    ])
}
