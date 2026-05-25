//! Full enum projection consumed by the per-language enum emitters.
//!
//! The shared `GeneratedEnum` is intentionally slim — only the fields the
//! build-script surface validator reads (`name`, `variants[].wire`). The
//! per-language emitters need every name (`rust_name`, `variant.rust`,
//! `variant.python`), so the bin loader builds this richer form directly
//! from the TOML surface spec.

use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub(super) struct EnumProjection {
    pub(super) name: String,
    pub(super) rust_name: String,
    pub(super) variants: Vec<EnumVariantProjection>,
}

#[derive(Debug, Clone)]
pub(super) struct EnumVariantProjection {
    pub(super) wire: String,
    pub(super) rust: String,
    pub(super) python: String,
}

#[derive(Debug, Deserialize)]
struct SurfaceSpecFragment {
    #[serde(default)]
    enums: Vec<SurfaceEnumFragment>,
}

#[derive(Debug, Deserialize)]
struct SurfaceEnumFragment {
    name: String,
    rust_name: String,
    variants: Vec<SurfaceEnumVariantFragment>,
}

#[derive(Debug, Deserialize)]
struct SurfaceEnumVariantFragment {
    wire: String,
    rust: String,
    python: String,
}

/// Read `endpoint_surface.toml` and project the `[enums]` block into the
/// emitter-facing form. Validates that Rust and Python identifiers are
/// unique per enum (wire-string uniqueness is enforced by the shared
/// validator already, but the bin-only Rust / Python projections add a
/// stricter check here so duplicates surface as the bin runs, not as a
/// language-toolchain failure downstream).
pub(super) fn load_enum_projections() -> Result<Vec<EnumProjection>, Box<dyn std::error::Error>> {
    let spec_path = "endpoint_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SurfaceSpecFragment = toml::from_str(&spec_str)?;

    let mut out = Vec::with_capacity(spec.enums.len());
    for enum_spec in spec.enums {
        let mut seen_rust = HashSet::new();
        let mut seen_python = HashSet::new();
        for variant in &enum_spec.variants {
            if !seen_rust.insert(variant.rust.clone()) {
                return Err(format!(
                    "enum '{}' has duplicate Rust variant '{}'",
                    enum_spec.name, variant.rust
                )
                .into());
            }
            if !seen_python.insert(variant.python.clone()) {
                return Err(format!(
                    "enum '{}' has duplicate Python member '{}'",
                    enum_spec.name, variant.python
                )
                .into());
            }
        }
        out.push(EnumProjection {
            name: enum_spec.name,
            rust_name: enum_spec.rust_name,
            variants: enum_spec
                .variants
                .into_iter()
                .map(|variant| EnumVariantProjection {
                    wire: variant.wire,
                    rust: variant.rust,
                    python: variant.python,
                })
                .collect(),
        });
    }
    Ok(out)
}
