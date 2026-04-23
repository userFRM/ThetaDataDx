//! Shared C-layout helpers for FPSS generator outputs.

use super::common::snake_case;
use super::schema::{sorted_data_events, ColumnDef, Schema};

#[derive(Clone, Copy)]
pub(super) struct CFieldLayout {
    pub(super) size: usize,
    pub(super) align: usize,
}

pub(super) fn c_field_layout(ty: &str) -> CFieldLayout {
    match ty {
        "i32" | "u8" => CFieldLayout { size: 4, align: 4 },
        "i64" | "u64" | "f64" | "Vec<u8>" => CFieldLayout { size: 8, align: 8 },
        "Contract" => CFieldLayout { size: 32, align: 8 },
        other => panic!("unsupported FPSS C field type for size generation: {other}"),
    }
}

pub(super) fn c_struct_size(columns: &[ColumnDef]) -> usize {
    c_layout(columns.iter().map(|column| c_field_layout(&column.r#type)))
}

pub(super) fn c_struct_offsets(columns: &[ColumnDef]) -> Vec<(String, usize)> {
    let mut offsets = Vec::with_capacity(columns.len());
    let mut size = 0;
    for column in columns {
        let field = c_field_layout(&column.r#type);
        size = align_to(size, field.align);
        offsets.push((column.name.clone(), size));
        size += field.size;
    }
    offsets
}

pub(super) fn fpss_event_size(schema: &Schema) -> usize {
    c_layout(
        fpss_event_fields(schema)
            .into_iter()
            .map(|(_, layout)| layout),
    )
}

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
    fields.push((String::from("control"), CFieldLayout { size: 16, align: 8 }));
    fields.push((
        String::from("raw_data"),
        CFieldLayout { size: 24, align: 8 },
    ));
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
