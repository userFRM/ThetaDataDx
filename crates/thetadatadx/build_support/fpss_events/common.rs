//! Shared helpers for per-language FPSS event renderers.
//!
//! Case conversion + per-language Rust type mapping for schema columns.

/// Convert a PascalCase event name to snake_case ("OpenInterest" → "open_interest")
/// so the `kind` discriminator exposed to Python matches the wire tag the
/// existing dict-based `next_event` emits.
pub(super) fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub(super) fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for ch in s.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.push(ch.to_ascii_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

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
        "Option<String>" => "Option<String>",
        "Option<i32>" => "Option<i32>",
        "Vec<u8>" => "Vec<u8>",
        // `Contract` becomes `Py<Contract>` on the pyclass — pyo3 cannot
        // expose a `Contract` struct by value through `#[pyo3(get)]` on a
        // frozen pyclass without a runtime acquisition, so we store the
        // Python handle directly. See `render_python_event_class_struct`.
        "Contract" => "Py<Contract>",
        other => {
            panic!("unsupported FPSS event column type '{other}' in {event_name}.{column_name}")
        }
    }
}

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
        "Option<String>" => "Option<String>",
        "Option<i32>" => "Option<i32>",
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

/// Returns `true` when the column needs a `BigInt::from(...)` wrapper in
/// the TS typed-event converter (i.e. crosses to JS as `bigint`).
pub(super) fn ts_needs_bigint(column_type: &str) -> bool {
    matches!(column_type, "i64" | "u64")
}

/// Returns `true` when the column is `Option<...>` on the Rust side and
/// therefore needs `Copy` handling in move/pattern bindings.
pub(super) fn is_option(column_type: &str) -> bool {
    column_type.starts_with("Option<")
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
        "Option<String>" => "Option<String>",
        "Option<i32>" => "Option<i32>",
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

// ── Shared helpers for the Rust-FFI / C header / Go emitters ────────────

/// Schema primitive → Rust `#[repr(C)]` scalar. The Rust-FFI struct names
/// each scalar exactly as the schema does; the C header mirror below uses
/// the matching `<cstdint>` alias so both sides have the same layout.
///
/// `Contract` is a nested `#[repr(C)]` struct emitted once per language
/// and embedded by value in every data event.
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
        "Contract" => "TdxContract",
        other => panic!(
            "unsupported Rust FFI column type '{other}' in {event_name}.{column_name} \
             (data variants must be pure scalars or Contract; strings/bytes belong on control/raw variants)"
        ),
    }
}

/// Zero literal for a schema primitive in the `ZERO_*` const body.
pub(super) fn rust_ffi_zero_literal(column_type: &str) -> &'static str {
    match column_type {
        "i32" | "i64" | "u64" | "u8" => "0",
        "f64" => "0.0",
        "Contract" => "ZERO_CONTRACT_STRUCT",
        other => panic!("no FFI zero literal for column type '{other}'"),
    }
}

/// Schema primitive → C `<stdint.h>` alias used by the cgo-facing header.
pub(super) fn c_ffi_scalar(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "int32_t",
        "i64" => "int64_t",
        "u64" => "uint64_t",
        "u8" => "uint8_t",
        "f64" => "double",
        "Contract" => "TdxContract",
        other => panic!("unsupported C FFI column type '{other}' in {event_name}.{column_name}"),
    }
}

/// Schema primitive → Go scalar (match the `C.` cgo type promotions the
/// generated converter uses — `int32`, `int64`, `uint64`, `uint8`,
/// `float64`).
pub(super) fn go_scalar(column_type: &str, event_name: &str, column_name: &str) -> &'static str {
    match column_type {
        "i32" => "int32",
        "i64" => "int64",
        "u64" => "uint64",
        "u8" => "uint8",
        "f64" => "float64",
        "Contract" => "*Contract",
        other => panic!("unsupported Go column type '{other}' in {event_name}.{column_name}"),
    }
}

/// True if a column's schema type is the structured `Contract` nested type.
pub(super) fn is_contract(column_type: &str) -> bool {
    column_type == "Contract"
}

/// snake_case column name → Go PascalCase field identifier.
///
/// Special-case: a bare `id` word maps to `ID` so `contract_id` →
/// `ContractID` (matching existing Go convention). Every other word is
/// simple `capitalize-first-letter-only`, so `ms_of_day` → `MsOfDay`,
/// `ext_condition1` → `ExtCondition1`, etc. Trailing digits stay attached
/// to the word they follow.
pub(super) fn snake_to_go_pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for part in s.split('_') {
        if part.is_empty() {
            continue;
        }
        if part == "id" {
            out.push_str("ID");
            continue;
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            for ch in chars {
                out.push(ch);
            }
        }
    }
    out
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
