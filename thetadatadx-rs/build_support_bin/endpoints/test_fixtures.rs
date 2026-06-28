//! Resolved fixture tables for the live-validator parameter-mode matrix.
//!
//! Bin-only: the build script never emits validator code so it doesn't
//! need this projection. Loaded directly from
//! `endpoint_surface.toml`'s `[test_fixtures]` block, side-stepping the
//! shared parser's TOML parse so this stays self-contained.

use std::collections::HashMap;

use serde::Deserialize;

/// Resolved fixture tables from the `[test_fixtures]` block, supplying
/// the live-validator parameter-mode matrix with sample argument values.
#[derive(Debug, Clone, Default)]
pub(super) struct TestFixtures {
    pub(super) category_symbol: HashMap<String, String>,
    pub(super) concrete_by_type: HashMap<String, String>,
    pub(super) concrete_overrides: HashMap<String, String>,
    pub(super) mode_overrides: HashMap<String, HashMap<String, String>>,
    pub(super) optional_defaults: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct SurfaceSpecFragment {
    test_fixtures: SurfaceTestFixturesFragment,
}

#[derive(Debug, Deserialize)]
struct SurfaceTestFixturesFragment {
    category_symbol: HashMap<String, String>,
    concrete_by_type: HashMap<String, String>,
    #[serde(default)]
    concrete_overrides: HashMap<String, String>,
    #[serde(default)]
    mode_overrides: HashMap<String, HashMap<String, String>>,
    optional_defaults: HashMap<String, String>,
}

/// Loads the `[test_fixtures]` block from `endpoint_surface.toml` into a
/// `TestFixtures` projection.
pub(super) fn load_test_fixtures() -> Result<TestFixtures, Box<dyn std::error::Error>> {
    let spec_path = "endpoint_surface.toml";
    let spec_str = std::fs::read_to_string(spec_path)?;
    let spec: SurfaceSpecFragment = toml::from_str(&spec_str)?;
    Ok(TestFixtures {
        category_symbol: spec.test_fixtures.category_symbol,
        concrete_by_type: spec.test_fixtures.concrete_by_type,
        concrete_overrides: spec.test_fixtures.concrete_overrides,
        mode_overrides: spec.test_fixtures.mode_overrides,
        optional_defaults: spec.test_fixtures.optional_defaults,
    })
}
