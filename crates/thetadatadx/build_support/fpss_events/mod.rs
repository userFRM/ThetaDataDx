//! FPSS streaming event schema → per-language typed struct generator.
//!
//! Reads `fpss_event_schema.toml` and emits:
//!
//! - `sdks/python/src/fpss_event_classes.rs` — `#[pyclass]` per Data variant
//!   + `buffered_event_to_typed` dispatcher + `register_fpss_event_classes`.
//! - `sdks/typescript/src/fpss_event_classes.rs` — `#[napi(object)]` per
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
//! * [`buffered`] — SSOT `BufferedEvent` enum emitted into both Python and
//!   TypeScript SDK crates.
//! * [`python`] / [`typescript`] — per-SDK typed event classes + dispatcher.
//! * [`ffi_rust`] — `#[repr(C)]` structs + converter for the Rust FFI crate.
//! * [`ffi_c`] — C mirror header `#include`'d from both Go cgo and C++ SDK.
//! * [`go_structs`] — Go-idiomatic public types + kind/control constants.

// Reason: the fpss_events/ tree is reached through two compilation
// contexts — `build.rs` (which only needs the schema loader + emitters
// for the baked-into-crate Rust FFI structs) and
// `bin/generate_sdk_surfaces` (which additionally uses the per-SDK
// renderers). Each context leaves the other half dead, so rather than
// carry two disjoint `cfg(...)` gates we silence the umbrella warnings
// here. Same pattern as `build_support/endpoints/mod.rs` +
// `build_support/ticks/mod.rs`.
#![allow(dead_code, unused_imports)]

use std::path::Path;

use super::ticks::GeneratedSourceFile;

mod buffered;
mod common;
mod ffi_c;
mod ffi_rust;
mod go_structs;
mod python;
mod schema;
mod typescript;

pub use typescript::ts_next_event_union_type;

pub fn write_sdk_generated_files(repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for file in render_sdk_generated_files()? {
        let abs = repo_root.join(file.relative_path);
        std::fs::write(&abs, &file.contents)?;
    }
    Ok(())
}

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
        GeneratedSourceFile {
            relative_path: "sdks/python/src/buffered_event.rs",
            contents: buffered.clone(),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/buffered_event.rs",
            contents: buffered,
        },
        GeneratedSourceFile {
            relative_path: "sdks/python/src/fpss_event_classes.rs",
            contents: python::render_python_fpss_event_classes(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/typescript/src/fpss_event_classes.rs",
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
        // `sdks/go/ffi_bridge.h` for Go cgo consumption AND from
        // `sdks/cpp/include/thetadx.h` for the C++ SDK. Same plain-C
        // typedefs serve both surfaces — keeping the schema as SSOT means
        // C++ can never drift from Rust in field order again (the old
        // hand-written C++ block diverged for months before #???).
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_event_structs.h.inc",
            contents: ffi_c::render_c_fpss_event_header(&schema),
        },
        GeneratedSourceFile {
            relative_path: "sdks/cpp/include/fpss_event_structs.h.inc",
            contents: ffi_c::render_c_fpss_event_header(&schema),
        },
        // Go-idiomatic struct definitions + kind enum + control constants
        // + `FpssEvent` wrapper. Standalone file in the `thetadatadx`
        // package, drop-in replacement for the hand-written block that
        // used to live inside `sdks/go/fpss.go`.
        GeneratedSourceFile {
            relative_path: "sdks/go/fpss_event_structs.go",
            contents: go_structs::render_go_fpss_event_structs(&schema),
        },
    ])
}
