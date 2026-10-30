//! The tick table, expanded into plain records for the writer.
//!
//! `thetadatadx::__ticks!` calls back into `tick_records!`, which resolves
//! each entry through `thetadatadx::__tick!` into a [`Tick`]. The layout of a
//! `Copy` row is read from the engine's own struct, so the C structs and the
//! C++ layout asserts hold to the layout the compiler gave the Rust row.

/// One tick type of the table.
pub(crate) struct Tick {
    pub(crate) name: &'static str,
    /// The first doc line without the field count, as Python and TypeScript
    /// render it.
    pub(crate) summary: &'static str,
    /// The doc lines after the first.
    pub(crate) more: &'static [&'static str],
    /// The row carries the `expiration`, `strike` and `right` a wildcard
    /// option response fills.
    pub(crate) contract_id: bool,
    pub(crate) columns: &'static [Column],
    pub(crate) flags: &'static [Flag],
    /// The C layout of a `Copy` row. An `#[owned]` row holds a `String` and
    /// has none.
    pub(crate) layout: Option<Layout>,
}

/// One column, in table order.
pub(crate) struct Column {
    /// The struct field.
    pub(crate) field: &'static str,
    /// The column type as the table writes it.
    pub(crate) ty: &'static str,
    pub(crate) doc: &'static str,
}

/// A boolean accessor, true when `field` is within `lo..=hi`.
pub(crate) struct Flag {
    pub(crate) name: &'static str,
    pub(crate) field: &'static str,
    pub(crate) lo: i32,
    pub(crate) hi: i32,
    pub(crate) doc: &'static str,
}

/// The size and alignment of a row and the offset of every field, in
/// declaration order: each column, followed by its presence flag when it is
/// nullable, then the contract fields.
pub(crate) struct Layout {
    pub(crate) size: usize,
    pub(crate) align: usize,
    pub(crate) offsets: &'static [(&'static str, usize)],
}

macro_rules! tick_records {
    ($($(#[$attr:ident])* $name:ident $body:tt)*) => {
        &[$(thetadatadx::__tick! { [$(#[$attr])* $name $body] => tick_record {} }),*]
    };
}

macro_rules! tick_record {
    ($name:ident $kind:ident contract [$($exp:ident $strike:ident $right:ident)?]
        doc $doc:tt summary [$($summary:tt)*] more [$($more:tt)*] required $required:tt
        columns [$({ $field:ident $wire:ident $ty:ident [$($has:ident)?] $d:tt })*]
        timestamps $timestamps:tt flags [$({ $flag:ident $ffield:ident $lo:tt $hi:tt $fdoc:tt })*]) => {
        Tick {
            name: stringify!($name),
            summary: concat!($($summary),*),
            more: &[$($more),*],
            contract_id: tick_record!(@contract_id $($exp)?),
            columns: &[$(Column { field: stringify!($field), ty: stringify!($ty), doc: $d }),*],
            flags: &[$(Flag { name: stringify!($flag), field: stringify!($ffield), lo: $lo, hi: $hi, doc: $fdoc }),*],
            layout: tick_record!(@layout $kind $name [$($field [$($has)?])* $($exp [] $strike [] $right [])?]),
        }
    };
    (@contract_id) => { false };
    (@contract_id $exp:ident) => { true };
    (@layout owned $name:ident $fields:tt) => { None };
    (@layout pod $name:ident [$($field:ident [$($has:ident)?])*]) => {
        Some(Layout {
            size: std::mem::size_of::<thetadatadx::$name>(),
            align: std::mem::align_of::<thetadatadx::$name>(),
            offsets: &[$(
                (stringify!($field), std::mem::offset_of!(thetadatadx::$name, $field)),
                $((stringify!($has), std::mem::offset_of!(thetadatadx::$name, $has)),)?
            )*],
        })
    };
}

/// Every tick type, in table order.
pub(crate) const TICKS: &[Tick] = thetadatadx::__ticks! { tick_records {} };

/// Every tick type, by name: the order the rendered files list them in.
pub(crate) fn ticks_by_name() -> Vec<&'static Tick> {
    let mut ticks: Vec<&Tick> = TICKS.iter().collect();
    ticks.sort_by_key(|tick| tick.name);
    ticks
}

/// The tick type a returned collection holds: the collection without its
/// plural `s`. Panics on a collection that names no tick type.
pub(crate) fn tick_for_collection(collection: &str) -> &'static Tick {
    TICKS
        .iter()
        .find(|tick| collection.strip_suffix('s') == Some(tick.name))
        .unwrap_or_else(|| panic!("collection '{collection}' names no tick type of the tick table"))
}
