//! Live integration tests against a locally-running ThetaTerminal.
//!
//! These tests issue real REST requests at the local Terminal (default
//! `http://127.0.0.1:25503`) and validate the response shape. They are
//! `#[ignore]`d by default so `cargo test` on a developer laptop with no
//! Terminal running stays green; opt in with:
//!
//! ```bash
//! THETADX_LIVE_LOCAL_TERMINAL=1 \
//!   cargo test --test test_rest_live -- --ignored
//! ```
//!
//! The corresponding unit tests in `crates/thetadatadx/src/rest/tests.rs`
//! cover the decoder contract (6-field NBBO subset + 11-field full
//! layouts, malformed-cell error surface, NaN reject) against synthetic
//! bodies; this module pins the wire-format expectations against an
//! actual Terminal so a Terminal-side regression doesn't slip past CI
//! silently.

use std::env;

use thetadatadx::rest::RestClient;

/// Env-gate name the runner checks before opting into live tests. Set
/// to any non-empty value to enable.
const LIVE_GATE: &str = "THETADX_LIVE_LOCAL_TERMINAL";

fn live_gate_enabled() -> bool {
    env::var(LIVE_GATE)
        .ok()
        .is_some_and(|v| !v.trim().is_empty())
}

fn live_base_url() -> String {
    env::var("THETADX_LIVE_TERMINAL_URL").unwrap_or_else(|_| "http://127.0.0.1:25503".to_string())
}

/// Smoke test: issue an `option_history_quote` against a known
/// historical date and confirm the response decodes to at least one
/// `QuoteTick` row.
#[tokio::test]
#[ignore = "live local Terminal required; set THETADX_LIVE_LOCAL_TERMINAL=1"]
async fn quote_history_decodes_known_historical_row() {
    if !live_gate_enabled() {
        return;
    }
    let rest = RestClient::new(live_base_url()).expect("RestClient::new");
    let ticks = rest
        .option_history_quote("QQQ", "20220415", "20220414")
        .strike("345")
        .right("call")
        .execute()
        .await
        .expect("live REST quote request");
    assert!(
        !ticks.is_empty(),
        "expected at least one QuoteTick row from the live Terminal"
    );
}

/// Sanity check: the cap surfaces before the decoder runs when the
/// caller sets it too low. Pinning the contract that
/// `RestClient::with_max_response_bytes` is enforced server-side path.
#[tokio::test]
#[ignore = "live local Terminal required; set THETADX_LIVE_LOCAL_TERMINAL=1"]
async fn quote_history_surfaces_response_too_large_under_tight_cap() {
    if !live_gate_enabled() {
        return;
    }
    let rest = RestClient::new(live_base_url())
        .expect("RestClient::new")
        .with_max_response_bytes(1); // 1-byte cap forces the surface.
    let result = rest
        .option_history_quote("QQQ", "20220415", "20220414")
        .strike("345")
        .right("call")
        .execute()
        .await;
    use thetadatadx::rest::RestError;
    match result {
        Err(RestError::ResponseTooLarge { size, limit }) => {
            assert_eq!(limit, 1);
            assert!(size > 1, "size should exceed the 1-byte cap");
        }
        other => panic!("expected ResponseTooLarge, got {other:?}"),
    }
}
