//! Regression: `DirectConfig::from_toml_str` (and the `[mdds]`/`[fpss]`/`[grpc]`
//! config-file section defaults behind it) must be INDEPENDENT of the ambient
//! environment.
//!
//! The section `Default` impls used to source `DirectConfig::production()`,
//! which applies the full `THETADATA_*` environment matrix. That had two
//! consequences a file-config caller never expects:
//!   1. a bad ambient selector (e.g. `THETADATA_HISTORICAL_TYPE=bogus`) made the
//!      env matrix panic, so parsing a TOML file panicked instead of returning
//!      an `Err` (or, here, instead of being irrelevant);
//!   2. an ambient `THETADATA_HISTORICAL_PORT` leaked into the parsed file
//!      config for a TOML that omitted the port.
//!
//! Sourcing `production_defaults()` (env-independent) fixes both.
//!
//! This lives in its own integration binary (separate process) and serialises
//! its two cases under one mutex so the process-global env mutation cannot race
//! the rest of the suite.

#![cfg(feature = "config-file")]

use std::sync::Mutex;

use thetadatadx::DirectConfig;

static ENV_GUARD: Mutex<()> = Mutex::new(());

#[test]
fn from_toml_str_ignores_ambient_env_for_panic_and_port() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    // Baseline read while the environment is clean (no THETADATA_HISTORICAL_*
    // set yet in this isolated test binary), so it equals the env-independent
    // production default. `production_defaults()` is crate-private, so derive it
    // through the public `production()` before mutating the env below.
    std::env::remove_var("THETADATA_HISTORICAL_TYPE");
    std::env::remove_var("THETADATA_HISTORICAL_PORT");
    let prod_port = DirectConfig::production().historical.port;

    // Case 1: a bad ambient historical selector must NOT panic `from_toml_str`.
    // The file config is env-independent, so a bogus selector is simply
    // irrelevant to parsing — no panic, a usable config back.
    std::env::set_var("THETADATA_HISTORICAL_TYPE", "definitely-not-a-real-env");
    let parsed = DirectConfig::from_toml_str("");
    std::env::remove_var("THETADATA_HISTORICAL_TYPE");
    assert!(
        parsed.is_ok(),
        "from_toml_str must not panic or fail on a bad ambient THETADATA_HISTORICAL_TYPE; \
         the file config is env-independent. got {parsed:?}",
    );

    // Case 2: an ambient port override must NOT leak into a TOML that omits the
    // port. The parsed historical port stays the env-independent default.
    let leaked_port: u16 = prod_port.wrapping_add(1);
    std::env::set_var("THETADATA_HISTORICAL_PORT", leaked_port.to_string());
    let cfg = DirectConfig::from_toml_str("").expect("empty TOML parses");
    std::env::remove_var("THETADATA_HISTORICAL_PORT");
    assert_eq!(
        cfg.historical.port, prod_port,
        "ambient THETADATA_HISTORICAL_PORT leaked into the file config for a TOML that omitted it",
    );
}
