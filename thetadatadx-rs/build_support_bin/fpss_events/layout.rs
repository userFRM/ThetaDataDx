//! Shared C-layout helpers for FPSS generator outputs.

use super::common::snake_case;
use super::schema::{sorted_control_events, sorted_data_events, ColumnDef, Schema};

/// Size and alignment, in bytes, of a single emitted C field.
#[derive(Clone, Copy)]
pub(super) struct CFieldLayout {
    pub(super) size: usize,
    pub(super) align: usize,
}

/// Layout of a single emitted C field, given the schema column's logical
/// type. `Vec<u8>` is the special case that splits into TWO physical C
/// fields: `<name>` (pointer, 8 bytes) and `<name>_len` (size_t, 8
/// bytes); the caller is expected to call `expand_columns` to materialise
/// the pair before computing offsets.
pub(super) fn c_field_layout(ty: &str) -> CFieldLayout {
    match ty {
        "u8" => CFieldLayout { size: 1, align: 1 },
        "i32" => CFieldLayout { size: 4, align: 4 },
        "i64" | "u64" | "f64" => CFieldLayout { size: 8, align: 8 },
        // String → `const char*` borrowed pointer.
        "String" => CFieldLayout { size: 8, align: 8 },
        "Contract" => CFieldLayout { size: 40, align: 8 },
        other => panic!("unsupported FPSS C field type for size generation: {other}"),
    }
}

/// Physical field layout of the shared `ThetaDataDxContract` struct, in
/// C declaration order. The `Contract` column type is opaque to
/// [`c_field_layout`] (it contributes a single 40-byte/align-8 slot to the
/// embedding event structs), so the contract's own internal fields are
/// modelled here once and consumed by both the size and per-field offset
/// helpers. Keeping this list as the single source means the asserted
/// numbers are computed by the same `next_multiple_of` walk that places every
/// other FPSS field, never hand-typed. Order and widths mirror the C mirror in
/// `ffi_c.rs::render_contract_struct_c` and the Rust `#[repr(C)]` struct in
/// `ffi_rust.rs::render_contract_struct_rust`.
fn contract_fields() -> [(&'static str, CFieldLayout); 9] {
    [
        // `const char* symbol` — borrowed NUL-terminated string pointer.
        ("symbol", CFieldLayout { size: 8, align: 8 }),
        ("sec_type", CFieldLayout { size: 4, align: 4 }),
        // Tagged-optional `bool has_*` flags + their payloads. C has no
        // `Option<T>`, so presence rides a leading `bool`.
        ("has_expiration", CFieldLayout { size: 1, align: 1 }),
        ("expiration", CFieldLayout { size: 4, align: 4 }),
        ("has_right", CFieldLayout { size: 1, align: 1 }),
        // `char right` — `'C'` / `'P'` (NUL when `has_right` is false).
        ("right", CFieldLayout { size: 1, align: 1 }),
        ("has_strike", CFieldLayout { size: 1, align: 1 }),
        ("strike", CFieldLayout { size: 8, align: 8 }),
        ("strike_thousandths", CFieldLayout { size: 4, align: 4 }),
    ]
}

/// Byte offset of each `ThetaDataDxContract` field, computed by the same
/// `next_multiple_of` walk as every other FPSS struct so the emitted asserts
/// pin the contract's internal layout directly rather than inferring it from
/// the embedding events.
pub(super) fn contract_field_offsets() -> Vec<(&'static str, usize)> {
    let fields = contract_fields();
    let mut offsets = Vec::with_capacity(fields.len());
    let mut size: usize = 0;
    for (name, layout) in fields {
        size = size.next_multiple_of(layout.align);
        offsets.push((name, size));
        size += layout.size;
    }
    offsets
}

/// Total size, in bytes, of the shared `ThetaDataDxContract` struct, computed
/// from [`contract_fields`]. Must equal the opaque 40-byte slot
/// [`c_field_layout`] reserves for an embedded `Contract` column.
pub(super) fn contract_size() -> usize {
    c_layout(contract_fields().into_iter().map(|(_, layout)| layout))
}

/// Expand a schema column list into the physical C field layout. The
/// only column type that splits into >1 physical fields is `Vec<u8>`
/// which becomes `(<name>: *const u8, <name>_len: size_t)`.
pub(super) fn expand_columns(columns: &[ColumnDef]) -> Vec<(String, CFieldLayout)> {
    let mut out = Vec::with_capacity(columns.len());
    for column in columns {
        if column.r#type == "Vec<u8>" {
            out.push((column.name.clone(), CFieldLayout { size: 8, align: 8 }));
            out.push((
                format!("{}_len", column.name),
                CFieldLayout { size: 8, align: 8 },
            ));
        } else {
            out.push((column.name.clone(), c_field_layout(&column.r#type)));
        }
    }
    out
}

/// Returns the total C struct size, in bytes, for a variant's columns, padded to at least one byte.
pub(super) fn c_struct_size(columns: &[ColumnDef]) -> usize {
    let expanded = expand_columns(columns);
    let size = c_layout(expanded.iter().map(|(_, l)| *l));
    // Empty C structs are not portable: GCC/Clang give them size 0,
    // MSVC gives them size 1. Rust `#[repr(C)] struct Foo;` is size 0.
    // The control variants emit a `_padding: u8` placeholder field on
    // both sides for unit variants, so callers should always see at
    // least one field.
    size.max(1)
}

/// Returns the byte offset of each physical C field for a variant's columns.
pub(super) fn c_struct_offsets(columns: &[ColumnDef]) -> Vec<(String, usize)> {
    let expanded = expand_columns(columns);
    let mut offsets = Vec::with_capacity(expanded.len());
    let mut size: usize = 0;
    for (name, layout) in &expanded {
        size = size.next_multiple_of(layout.align);
        offsets.push((name.clone(), size));
        size += layout.size;
    }
    offsets
}

/// Alignment requirement of any emitted event struct, computed from the
/// widest field in its expanded column list. An empty column list is a
/// `_padding: u8` placeholder, hence align 1. Shared by the Rust-side
/// `align_of!` asserts and the control-variant placement walk so the
/// asserted alignment is never hand-typed.
pub(super) fn struct_align(columns: &[ColumnDef]) -> usize {
    if columns.is_empty() {
        1
    } else {
        expand_columns(columns)
            .iter()
            .map(|(_, l)| l.align)
            .max()
            .unwrap_or(1)
    }
}

/// Alignment requirement of the shared `ThetaDataDxContract` struct,
/// computed from its widest field so the Rust-side `align_of!` assert
/// tracks the same layout walk that places every embedding event.
pub(super) fn contract_align() -> usize {
    contract_fields()
        .iter()
        .map(|(_, l)| l.align)
        .max()
        .unwrap_or(1)
}

/// Returns the total C struct size, in bytes, of the tagged `ThetaDataDxStreamEvent` wrapper.
pub(super) fn fpss_event_size(schema: &Schema) -> usize {
    c_layout(
        fpss_event_fields(schema)
            .into_iter()
            .map(|(_, layout)| layout),
    )
}

/// Returns the byte offset of each field on the tagged `ThetaDataDxStreamEvent` wrapper.
pub(super) fn fpss_event_offsets(schema: &Schema) -> Vec<(String, usize)> {
    let mut offsets = Vec::new();
    let mut size: usize = 0;
    for (field_name, layout) in fpss_event_fields(schema) {
        size = size.next_multiple_of(layout.align);
        offsets.push((field_name, size));
        size += layout.size;
    }
    offsets
}

/// Alignment requirement of the tagged `ThetaDataDxStreamEvent` wrapper,
/// taken as the widest embedded field so the Rust-side `align_of!` assert
/// tracks the same walk that sizes the wrapper.
pub(super) fn fpss_event_align(schema: &Schema) -> usize {
    fpss_event_fields(schema)
        .into_iter()
        .map(|(_, layout)| layout.align)
        .max()
        .unwrap_or(1)
}

fn fpss_event_fields(schema: &Schema) -> Vec<(String, CFieldLayout)> {
    let mut fields = Vec::new();
    fields.push((String::from("kind"), CFieldLayout { size: 4, align: 4 }));
    for (event_name, def) in sorted_data_events(schema) {
        fields.push((
            snake_case(event_name),
            CFieldLayout {
                size: c_struct_size(&def.columns),
                align: 8,
            },
        ));
    }
    // Per-variant typed control payloads — one struct per
    // `kind = "control"` schema entry, embedded by value. The schema
    // ordering matches `sorted_control_events`.
    for (event_name, def) in sorted_control_events(schema) {
        fields.push((
            snake_case(event_name),
            CFieldLayout {
                size: c_struct_size(&def.columns),
                align: struct_align(&def.columns),
            },
        ));
    }
    fields
}

fn c_layout(fields: impl IntoIterator<Item = CFieldLayout>) -> usize {
    let mut size: usize = 0;
    let mut struct_align = 1;
    for field in fields {
        struct_align = struct_align.max(field.align);
        size = size.next_multiple_of(field.align) + field.size;
    }
    size.next_multiple_of(struct_align)
}
