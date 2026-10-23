//! Regression: `DirectConfig::from_toml_str` (and the `[mdds]`/`[fpss]`/`[grpc]`
//! config-file section defaults behind it) must be INDEPENDENT of the ambient
//! environment.
//!
//! The section `Default` impls used to source `DirectConfig::production()`,
//! which applies the full `THETADATA_*` environment matrix. That had two
//! consequences a file-config caller never expects:
//!   1. a bad ambient selector (e.g. `THETADATA_MARKET_DATA_TYPE=bogus`) made the
//!      env matrix panic, so parsing a TOML file panicked instead of returning
//!      an `Err` (or, here, instead of being irrelevant);
//!   2. an ambient `THETADATA_MARKET_DATA_PORT` leaked into the parsed file
//!      config for a TOML that omitted the port.
//!
//! Sourcing `production_defaults()` (env-independent) fixes both.
//!
//! This lives in its own integration binary (separate process) holding this
//! one test, so its process-global env mutation cannot race the rest of the
//! suite.

#![cfg(feature = "config-file")]

use thetadatadx::DirectConfig;

/// Set `key` to `value`, or remove it when `value` is `None`.
fn set_env(key: &str, value: Option<&str>) {
    // SAFETY: this integration binary holds a single test, so no other thread
    // reads or writes the process environment while it changes.
    unsafe {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn from_toml_str_ignores_ambient_env_for_panic_and_port() {
    // Baseline read while the environment is clean (no THETADATA_MARKET_DATA_*
    // set yet in this isolated test binary), so it equals the env-independent
    // production default. `production_defaults()` is crate-private, so derive it
    // through the public `production()` before mutating the env below.
    set_env("THETADATA_MARKET_DATA_TYPE", None);
    set_env("THETADATA_MARKET_DATA_PORT", None);
    let prod_port = DirectConfig::production().market_data.port;

    // Case 1: a bad ambient market-data selector must NOT panic `from_toml_str`.
    // The file config is env-independent, so a bogus selector is simply
    // irrelevant to parsing — no panic, a usable config back.
    set_env(
        "THETADATA_MARKET_DATA_TYPE",
        Some("definitely-not-a-real-env"),
    );
    let parsed = DirectConfig::from_toml_str("");
    set_env("THETADATA_MARKET_DATA_TYPE", None);
    assert!(
        parsed.is_ok(),
        "from_toml_str must not panic or fail on a bad ambient THETADATA_MARKET_DATA_TYPE; \
         the file config is env-independent. got {parsed:?}",
    );

    // Case 2: an ambient port override must NOT leak into a TOML that omits the
    // port. The parsed market-data port stays the env-independent default.
    let leaked_port: u16 = prod_port.wrapping_add(1);
    set_env("THETADATA_MARKET_DATA_PORT", Some(&leaked_port.to_string()));
    let cfg = DirectConfig::from_toml_str("").expect("empty TOML parses");
    set_env("THETADATA_MARKET_DATA_PORT", None);
    assert_eq!(
        cfg.market_data.port, prod_port,
        "ambient THETADATA_MARKET_DATA_PORT leaked into the file config for a TOML that omitted it",
    );
}
