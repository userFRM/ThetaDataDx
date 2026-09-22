//! TOML-backed schema types for `tick_schema.toml` reachable from the
//! `generate_sdk_surfaces` binary.
//!
//! Adds the per-language render block + doc / copy / align metadata that
//! the SDK projection emitters need on top of the build-script's
//! decoder-focused view. Both compile units deserialize the same TOML —
//! extra fields here, missing fields in the build-script schema.

use std::collections::HashMap;

use serde::Deserialize;

/// Deserialized `tick_schema.toml`: every tick type keyed by its schema name.
#[derive(Debug, Deserialize)]
pub(crate) struct Schema {
    pub(crate) types: HashMap<String, TickTypeDef>,
}

/// Parsed from `tick_schema.toml`. `doc`, `copy`, and `align` drive the
/// generated tick struct layout; `render` drives the per-language binding
/// name lookups consumed by every renderer in
/// `build_support_bin/endpoints/sdk_render/` and `build_support_bin/ticks`.
#[derive(Debug, Deserialize)]
pub(crate) struct TickTypeDef {
    pub(crate) doc: String,
    pub(crate) copy: bool,
    #[serde(default)]
    pub(crate) align: Option<u32>,
    /// Wire decoder function name. Build-script-only — the bin emitters
    /// never see decoded ticks, only their schema. Underscore-prefixed
    /// so the lint allows the field on the bin compile unit.
    #[serde(rename = "parser")]
    _parser: String,
    /// Wire decoder required-fields list. Build-script-only on the same
    /// rationale as `_parser`.
    #[serde(default, rename = "required")]
    _required: Vec<String>,
    /// Wire decoder EOD-row layout flag. Build-script-only on the same
    /// rationale as `_parser`.
    #[serde(default, rename = "eod_style")]
    _eod_style: bool,
    #[serde(default)]
    pub(crate) contract_id: bool,
    pub(crate) columns: Vec<ColumnDef>,
    /// Declarative boolean flag-word accessors decoded from the tick's
    /// integer flag / condition columns (e.g. `is_cancelled`,
    /// `is_cancelled`). The Rust core hand-writes these in
    /// `thetadatadx-rs/src/tdbe/types/tick.rs`; this list is the single source
    /// the SDK emitters project into Python (computed `#[getter]`),
    /// TypeScript (precomputed `#[napi(object)]` field), and C++ (a free
    /// function in `thetadatadx.hpp`) so a binding caller never hand-decodes
    /// `condition_flags` / `price_flags`. Empty for most tick types.
    #[serde(default)]
    pub(crate) flag_accessors: Vec<FlagAccessorDef>,
    /// Per-language binding name map. Populated for every tick type so
    /// the SDK projection emitters reach for one TOML row per tick type.
    pub(crate) render: TickRenderDef,
}

/// One boolean accessor decoded from a tick's integer flag columns. The
/// `kind` selects the predicate shape; each emitter renders the
/// idiomatic form for its language from the same operands so the three
/// bindings agree by construction with the Rust core.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct FlagAccessorDef {
    /// Accessor name in snake_case (Python getter / C++ free function);
    /// the TypeScript emitter camelCases it for the object key.
    pub(crate) name: String,
    /// One-sentence description rendered as the accessor's doc comment.
    pub(crate) doc: String,
    /// Source column the predicate reads (must be an `i32` column on the
    /// owning tick type).
    pub(crate) field: String,
    /// Predicate shape — `"range_inclusive"`, `"bit_set"`, or `"eq"`.
    pub(crate) kind: String,
    /// `range_inclusive`: inclusive lower bound. Unused by other kinds.
    ///
    /// Optional in the type and required by kind: a missing bound used to
    /// default to zero, and zero is a bound a predicate can act on. Omitting
    /// `lo` from the cancellation range silently widened it from `40..=44` to
    /// `0..=44`, which covers `REGULAR`, so every ordinary trade would have
    /// reported itself cancelled on four language surfaces.
    #[serde(default)]
    pub(crate) lo: Option<i32>,
    /// `range_inclusive`: inclusive upper bound. Unused by other kinds.
    #[serde(default)]
    pub(crate) hi: Option<i32>,
    /// `bit_set`: bitmask tested with `field & mask == mask`.
    /// `eq`: the value `field` is compared equal to. Unused by
    /// `range_inclusive`.
    ///
    /// Optional in the type and required by kind: a missing mask defaulted to
    /// zero, and `field & 0 == 0` is true for every input, so the accessor
    /// became a predicate that cannot be false.
    #[serde(default)]
    pub(crate) value: Option<i32>,
}

impl FlagAccessorDef {
    /// The inclusive bounds of a `range_inclusive` predicate.
    ///
    /// Panics when either is missing. The bound a predicate acts on cannot be
    /// defaulted: zero is a code the vendor sends, so a missing `lo` would
    /// quietly widen the range over it on four language surfaces at once.
    fn range_bounds(&self) -> (i32, i32) {
        match (self.lo, self.hi) {
            (Some(lo), Some(hi)) => {
                assert!(
                    lo <= hi,
                    "flag_accessor '{}': range_inclusive has lo {lo} above hi {hi}",
                    self.name
                );
                (lo, hi)
            }
            _ => panic!(
                "flag_accessor '{}': range_inclusive needs both `lo` and `hi`",
                self.name
            ),
        }
    }

    /// The operand of a `bit_set` or `eq` predicate.
    ///
    /// Panics when missing. A defaulted mask makes `field & 0 == 0`, which is
    /// true for every input: an accessor that cannot be false.
    fn operand(&self) -> i32 {
        self.value
            .unwrap_or_else(|| panic!("flag_accessor '{}': {} needs `value`", self.name, self.kind))
    }

    /// Render the predicate as a boolean Rust expression over `cell`, an
    /// `i32` expression naming the column the predicate reads. The caller
    /// supplies it, so a surface that carries the column as `Option<i32>`
    /// can test the unwrapped cell. Shared by the Python and TypeScript
    /// emitters, which both generate Rust source.
    pub(crate) fn rust_predicate(&self, cell: &str) -> String {
        match self.kind.as_str() {
            "range_inclusive" => {
                let (lo, hi) = self.range_bounds();
                format!("({lo}..={hi}).contains(&{cell})")
            }
            "bit_set" => {
                let mask = self.operand();
                format!("{cell} & {mask} == {mask}")
            }
            "eq" => {
                let value = self.operand();
                format!("{cell} == {value}")
            }
            other => panic!(
                "unsupported flag_accessor kind '{other}' for '{}'; expected range_inclusive / bit_set / eq",
                self.name
            ),
        }
    }

    /// Render the predicate as a boolean C++ expression over `c.<field>`
    /// (the streaming/tick struct argument named `c`).
    pub(crate) fn cpp_predicate(&self, src: &str) -> String {
        let field = &self.field;
        match self.kind.as_str() {
            "range_inclusive" => {
                let (lo, hi) = self.range_bounds();
                format!("{src}.{field} >= {lo} && {src}.{field} <= {hi}")
            }
            "bit_set" => {
                let mask = self.operand();
                format!("({src}.{field} & {mask}) == {mask}")
            }
            "eq" => {
                let value = self.operand();
                format!("{src}.{field} == {value}")
            }
            other => panic!(
                "unsupported flag_accessor kind '{other}' for '{}'; expected range_inclusive / bit_set / eq",
                self.name
            ),
        }
    }
}

/// One column on a tick type: its wire spelling, public field name, schema type tag, and optional docs-site description.
#[derive(Debug, Deserialize)]
pub(crate) struct ColumnDef {
    /// Wire / decode-layer column spelling. Every public surface emits
    /// `field`; only the docs generator still prints `name` (in its
    /// missing-doc diagnostics), so the field is dead code in the
    /// `generate_sdk_surfaces` compile unit — same gate as `doc`.
    #[cfg_attr(not(feature = "__internal"), allow(dead_code))]
    pub(crate) name: String,
    pub(crate) field: String,
    pub(crate) r#type: String,
    /// One-sentence field description rendered on the docs-site
    /// response-schema tables (`generate_docs_site`). Optional at the
    /// serde layer so the build-script view of the same TOML stays
    /// untouched; the docs generator fails loudly on a missing doc.
    /// Only the docs generator reads it, so the field is dead code in
    /// the `generate_sdk_surfaces` compile unit (no `__internal`).
    /// Whether an absent cell must be carried as "no value" rather than
    /// filled with the column's zero.
    ///
    /// Most columns can fill: a null size or count is absent data, and zero
    /// reads the same way. These cannot. The vendor assigns zero a meaning on
    /// every condition and exchange column — quote condition 0 is `REGULAR`,
    /// a firm two-sided quote, and exchange 0 is the composite — so filling a
    /// wire null with zero publishes a positive assertion the vendor never
    /// made, and nothing downstream can tell the two apart. The vendor's own
    /// client renders such a cell as null rather than as a code.
    #[serde(default)]
    pub(crate) nullable: bool,
    #[serde(default)]
    #[cfg_attr(not(feature = "__internal"), allow(dead_code))]
    pub(crate) doc: Option<String>,
}

/// The per-language binding names the per-tick emitters in this tree read
/// for one tick type: the pyclass / Vec / Arrow projections and the
/// TypeScript class vec. The other binding names in the same TOML row
/// (FFI / C++ / direct) are read by the endpoint emitters' `sdk_helpers`,
/// which reparses the TOML independently; serde ignores the keys this
/// struct does not name, so they are not redeclared here.
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct TickRenderDef {
    /// Wire-collection plural keying every renderer call (e.g. `"GreeksTicks"`).
    /// Matches the `returns` value declared on each endpoint in
    /// `endpoint_surface.toml`. Build-time validator rejects duplicates and
    /// strays.
    pub(crate) collection: String,
    pub(crate) python_pyclass_list: String,
    pub(crate) python_vec_to_pylist: String,
    pub(crate) python_slice_arrow: String,
    pub(crate) ts_class_vec: String,
    pub(crate) pyclass: String,
}

/// Loads and parses `tick_schema.toml` from the current working directory into a [`Schema`].
pub(crate) fn load_schema() -> Result<Schema, Box<dyn std::error::Error>> {
    let schema_path = "tick_schema.toml";
    let schema_str = std::fs::read_to_string(schema_path)?;
    let schema: Schema = toml::from_str(&schema_str)?;
    Ok(schema)
}

/// Borrow the render block of a schema type by name. Panics with the
/// available keys when the type is missing -- a missing tick type is a
/// build-time bug. Used by every ticks/* emitter that previously kept a
/// hand-coded match arm per tick type for FFI / Python / TS binding
/// names.
pub(crate) fn render_for_type<'a>(schema: &'a Schema, type_name: &str) -> &'a TickRenderDef {
    schema
        .types
        .get(type_name)
        .map(|d| &d.render)
        .unwrap_or_else(|| {
            let mut keys: Vec<&str> = schema.types.keys().map(String::as_str).collect();
            keys.sort();
            panic!(
                "no render block for tick type '{type_name}' in tick_schema.toml; available: {}",
                keys.join(", ")
            )
        })
}
