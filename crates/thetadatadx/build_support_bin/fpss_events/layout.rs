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
        "Contract" => CFieldLayout { size: 32, align: 8 },
        other => panic!("unsupported FPSS C field type for size generation: {other}"),
    }
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
    let mut size = 0;
    for (name, layout) in &expanded {
        size = align_to(size, layout.align);
        offsets.push((name.clone(), size));
        size += layout.size;
    }
    offsets
}

/// Alignment requirement of a control-variant struct, computed from its
/// columns. Empty-column control variants fall back to align=1 (one
/// `_padding` byte).
pub(super) fn control_struct_align(columns: &[ColumnDef]) -> usize {
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
    let mut size = 0;
    for (field_name, layout) in fpss_event_fields(schema) {
        size = align_to(size, layout.align);
        offsets.push((field_name, size));
        size += layout.size;
    }
    offsets
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
                align: control_struct_align(&def.columns),
            },
        ));
    }
    fields
}

fn c_layout(fields: impl IntoIterator<Item = CFieldLayout>) -> usize {
    let mut size = 0;
    let mut struct_align = 1;
    for field in fields {
        struct_align = struct_align.max(field.align);
        size = align_to(size, field.align) + field.size;
    }
    align_to(size, struct_align)
}

fn align_to(value: usize, align: usize) -> usize {
    let rem = value % align;
    if rem == 0 {
        value
    } else {
        value + align - rem
    }
}
