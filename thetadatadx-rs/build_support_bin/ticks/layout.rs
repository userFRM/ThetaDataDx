//! Tick FFI layout computation — `(field, offset)` pairs and `(size, align)`
//! for each `#[repr(C, align(N))]` tick struct described in
//! `tick_schema.toml`.
//!
//! Used by the C++ layout-assert emitter (`build_support/ticks/cpp.rs`) and
//! the tdbe layout-assert emitter (`build_support/ticks/tdbe_structs.rs`)
//! to verify Rust ↔ C++ ↔ tdbe struct ABI agreement at build time.

use super::schema::TickTypeDef;

/// `(field, byte_offset)` pairs for every column the parser fills, in
/// declaration order, including the `contract_id` triple and
/// `QuoteTick.midpoint` tail.
pub(super) fn tick_ffi_offsets(type_name: &str, def: &TickTypeDef) -> Vec<(String, usize)> {
    let mut offsets = Vec::new();
    let mut size = 0usize;
    for (field_name, field_type) in tick_ffi_fields(type_name, def) {
        let (field_size, field_align) = tick_ffi_field_layout(field_type);
        size = size.next_multiple_of(field_align);
        offsets.push((field_name.to_string(), size));
        size += field_size;
    }
    offsets
}

/// Compute `(size, alignment)` of an FFI tick struct from its
/// `tick_schema.toml` row. Mirrors `#[repr(C, align(N))]` — alignment is
/// the max of the schema's `align` directive and every field's natural
/// alignment, and size is rounded up to a multiple of that alignment to
/// reproduce Rust's struct tail padding.
pub(super) fn tick_ffi_size_and_align(type_name: &str, def: &TickTypeDef) -> (usize, usize) {
    let mut size = 0usize;
    let mut struct_align = def.align.unwrap_or(1) as usize;
    for (_, field_type) in tick_ffi_fields(type_name, def) {
        let (field_size, field_align) = tick_ffi_field_layout(field_type);
        struct_align = struct_align.max(field_align);
        size = size.next_multiple_of(field_align) + field_size;
    }
    (size.next_multiple_of(struct_align), struct_align)
}

fn tick_ffi_fields<'a>(type_name: &'a str, def: &'a TickTypeDef) -> Vec<(&'a str, &'a str)> {
    let mut fields = def
        .columns
        .iter()
        .map(|column| (column.field.as_str(), column.r#type.as_str()))
        .collect::<Vec<_>>();
    if def.contract_id {
        fields.push(("expiration", "i32"));
        fields.push(("strike", "price"));
        fields.push(("right", "right"));
    }
    if type_name == "QuoteTick" {
        fields.push(("midpoint", "price"));
    }
    fields
}

fn tick_ffi_field_layout(kind: &str) -> (usize, usize) {
    match kind {
        // `right` is a Rust `char` / C `uint32_t` (Unicode scalar value
        // of 'C' / 'P') — same 4-byte slot the wire's i32 occupied.
        // `calendar_status` is a `repr(i32)` enum / C `int32_t`.
        "i32" | "eod_num" | "eod_date" | "right" | "calendar_status" => (4, 4),
        "i64" | "eod_num64" | "f64" | "price" | "eod_price" => (8, 8),
        // Rust `bool` / C99 `bool`: one byte, one-byte alignment.
        "bool" => (1, 1),
        "String" => (
            std::mem::size_of::<*const ()>(),
            std::mem::align_of::<*const ()>(),
        ),
        other => panic!("unsupported tick FFI field type: {other}"),
    }
}
