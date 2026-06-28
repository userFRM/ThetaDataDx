//! Shared helpers for per-language FPSS event renderers.
//!
//! Case conversion + per-language Rust type mapping for schema columns.

use heck::ToSnakeCase;

/// Convert a PascalCase event name to snake_case ("OpenInterest" → "open_interest")
/// so the `kind` discriminator exposed to Python matches the wire tag the
/// existing dict-based `next_event` emits.
pub(super) fn snake_case(name: &str) -> String {
    name.to_snake_case()
}

/// Core `StreamControl` variant backing a schema control event. The two
/// names coincide for every variant except `ParseError`, whose core
/// enum variant keeps the historical `Error` spelling — the public
/// event class is named `ParseError` so no binding ships a class that
/// collides with the language's own error types.
pub(super) fn control_rust_variant(event_name: &str) -> &str {
    match event_name {
        "ParseError" => "Error",
        other => other,
    }
}

/// Maps a schema column type to the Rust field type emitted on the Python `#[pyclass]` struct.
pub(super) fn python_rust_field_type(
    column_type: &str,
    event_name: &str,
    column_name: &str,
) -> &'static str {
    match column_type {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "u8" => "u8",
        "f64" => "f64",
        "String" => "String",
        "Vec<u8>" => "Vec<u8>",
        // `Contract` becomes `Py<ContractRef>` on the pyclass — pyo3 cannot
        // expose a struct by value through `#[pyo3(get)]` on a frozen
        // pyclass without a runtime acquisition, so we store the Python
        // handle directly. The Python-side struct is named `ContractRef`
        // to disambiguate from the fluent `Contract` builder registered
        // by `fluent.rs`. See `render_python_event_class_struct`.
        "Contract" => "Py<ContractRef>",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

/// Maps a schema column type to the Rust field type emitted on the TypeScript `#[napi(object)]` struct.
pub(super) fn ts_rust_field_type(
    column_type: &str,
    event_name: &str,
    column_name: &str,
) -> &'static str {
    match column_type {
        "i32" => "i32",
        // Epoch nanoseconds today are ~1.8e18, well past
        // `Number.MAX_SAFE_INTEGER` (2^53 - 1 ≈ 9e15). `i64`/`u64` fields
        // must cross to JS as `bigint` to avoid silent precision loss.
        // `napi::bindgen_prelude::BigInt` is how napi-rs 3.x exposes that.
        "u64" | "i64" => "BigInt",
        // `u8` is widened to `u32` because napi-rs has no `u8` encoder
        // (JS numbers are f64-backed; sub-32-bit ints are all the same
        // on the wire).
        "u8" => "u32",
        "f64" => "f64",
        "String" => "String",
        "Vec<u8>" => "Vec<u8>",
        // `Contract` lowers to a nested `#[napi(object)]` struct —
        // emitted once at the top of `fpss_event_classes.rs` and
        // embedded by value in every data event.
        "Contract" => "Contract",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

/// Field type emitted in the shared `BufferedEvent` enum. Native Rust
/// types — the FFI widening (BigInt for i64/u64, etc.) happens in the
/// per-language typed dispatcher.
pub(super) fn rust_field_type(
    column_type: &str,
    event_name: &str,
    column_name: &str,
) -> &'static str {
    match column_type {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "u8" => "u8",
        "f64" => "f64",
        "String" => "String",
        "Vec<u8>" => "Vec<u8>",
        // `Contract` on the `BufferedEvent` carries the full contract by
        // value so the per-language dispatcher does not need to
        // re-resolve it. `std::sync::Arc<fpss::protocol::Contract>`
        // would be marginally cheaper to clone but the language
        // dispatchers already construct new pyclass / napi objects from
        // the contract fields anyway — cloning the wrapped Contract
        // once on buffer is a single heap alloc amortised over the
        // whole event.
        "Contract" => "fpss::protocol::Contract",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

// ── Shared helpers for the Rust-FFI / C header emitters ─────────────────

/// Schema primitive → Rust `#[repr(C)]` scalar. The Rust-FFI struct names
/// each scalar exactly as the schema does; the C header mirror below uses
/// the matching `<cstdint>` alias so both sides have the same layout.
///
/// `Contract` is a nested `#[repr(C)]` struct emitted once per language
/// and embedded by value in every data event. `String` / `Vec<u8>`
/// only appear on `kind = "control"` variants (e.g. `UnknownFrame`,
/// `Ping`) and are represented as borrowed C pointers — the backing
/// storage lives on the `FfiBufferedEvent` wrapper alongside the event
/// for the duration of the user callback.
pub(super) fn rust_ffi_scalar(
    column_type: &str,
    event_name: &str,
    column_name: &str,
) -> &'static str {
    match column_type {
        "i32" => "i32",
        "i64" => "i64",
        "u64" => "u64",
        "u8" => "u8",
        "f64" => "f64",
        "Contract" => "ThetaDataDxContract",
        // String → borrowed C string, backed by an Option<CString> on
        // `FfiBufferedEvent`. Null when the source variant has no
        // string payload (zero-fill case for inactive variants).
        "String" => "*const c_char",
        // Vec<u8> emits as a (ptr, len) pair via the dedicated
        // `rust_ffi_emit_struct_field` path; this scalar mapping is
        // unreachable because the column expansion handles the pair.
        other => panic!(
            "unsupported Rust FFI scalar mapping for column type '{other}' \
             in {event_name}.{column_name}"
        ),
    }
}

/// Zero literal for a schema primitive in the `ZERO_*` const body.
pub(super) fn rust_ffi_zero_literal(column_type: &str) -> &'static str {
    match column_type {
        "i32" | "i64" | "u64" | "u8" => "0",
        "f64" => "0.0",
        "Contract" => "ZERO_CONTRACT_STRUCT",
        "String" => "ptr::null()",
        other => panic!("no FFI zero literal for column type '{other}'"),
    }
}

/// Schema primitive → C `<stdint.h>` alias used by the C-ABI header.
pub(super) fn c_ffi_scalar(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "int32_t",
        "i64" => "int64_t",
        "u64" => "uint64_t",
        "u8" => "uint8_t",
        "f64" => "double",
        "Contract" => "ThetaDataDxContract",
        // Borrowed C string, NUL-terminated. May be null on inactive
        // variants. Never freed by the consumer.
        "String" => "const char *",
        other => panic!(
            "unsupported C FFI scalar mapping for column type '{other}' \
             in {event_name}.{column_name}"
        ),
    }
}

/// True when the column's wire type expands to a `(*const u8, size_t)`
/// pair on both the C and Rust FFI sides. The schema column carries one
/// logical name; the emitted struct gets `<name>` (pointer) and
/// `<name>_len` (size_t).
pub(super) fn is_byte_buffer(column_type: &str) -> bool {
    column_type == "Vec<u8>"
}

/// True if a column's schema type is the structured `Contract` nested type.
pub(super) fn is_contract(column_type: &str) -> bool {
    column_type == "Contract"
}

/// `Quote` → `ZERO_QUOTE`, `OpenInterest` → `ZERO_OI`, `Ohlcvc` →
/// `ZERO_OHLCVC`. Matches the hand-written names the converter used so
/// diffs against the old code stay readable.
pub(super) fn zero_const_name(event_name: &str) -> String {
    match event_name {
        "OpenInterest" => "ZERO_OI".to_string(),
        _ => format!("ZERO_{}", snake_case(event_name).to_uppercase()),
    }
}
