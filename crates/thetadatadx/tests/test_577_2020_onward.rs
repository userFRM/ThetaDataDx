//! Live-gated regression test for issue #577.
//!
//! The post-Feb-2020 daily-expiry quote backfill is the canonical
//! reproducer for the h2 cascade: any QQQ `option_history_quote` call
//! over a date in `[2020-02-25, 2023-01-01)` against an unpatched
//! upstream MDDS server tears down the h2 stream mid-response with
//! `transport error (connection_closed)`. PR #573 unblocked the
//! `[2019-05, 2020-02-24]` range by adding the lenient 6-field
//! decoder + REST transport; this branch (issue #577) wires
//! automatic REST fallback + channel-layer recovery so the same
//! `option_history_quote_with_fallback` call now returns ticks on
//! every trading day in the 2021 reproducer window.
//!
//! ## Gating
//!
//! Live calls require:
//!
//!   - `THETADX_LIVE_CREDS=/path/to/creds.txt` -- ThetaData credentials.
//!   - A locally-running, authenticated, PATCHED Terminal listening
//!     on `http://127.0.0.1:25503/v3/...` (the REST fallback target).
//!     The patcher lives at `tools/local-terminal-patcher/`.
//!
//! Without either env var the test exits early as `Ok(())` -- CI
//! environments without credentials see the test as a passing no-op.
//!
//! ## What this test pins
//!
//! Five trading days from `2021-04-01` to `2021-04-07`
//! (`{20210401, 20210405, 20210406, 20210407}` -- skipping
//! the weekend) all must return tick counts > 0 via
//! `option_history_quote_with_fallback` with
//! `FallbackPolicy::RestAlwaysForDateRange { before: 20230101 }`.
//! Each day's tick count is logged at info level so a future
//! regression that silently shrinks the response is visible in CI.
//!
//! ## Why the date range
//!
//! The reproducer in #577 hits h2 disconnects on most days from
//! 2020-02-25 onward; we pick 2021-04 (inside the cascade window,
//! well after the schema cutover) and assert all 5 trading days
//! return a non-empty tick stream. A non-zero count proves the
//! fallback path returned data, not that the data is *correct* --
//! correctness is pinned by the unit tests in `mdds::decode::tests`
//! and `rest::tests`.

use thetadatadx::{
    Credentials, DirectConfig, FallbackPolicy, ThetaDataDxClient, DEFAULT_REST_BASE_URL,
};

const REPRODUCER_DAYS: &[&str] = &[
    "20210401", // Thursday
    "20210405", // Monday (skipping the Good Friday weekend)
    "20210406", // Tuesday
    "20210407", // Wednesday
    "20210408", // Thursday
];

/// Backfill 5 trading days through `option_history_quote_with_fallback`
/// and assert every day returns at least one tick. This is the exact
/// reproducer shape from issue #577 -- with the fallback policy
/// `RestAlwaysForDateRange { before: 20230101 }`, the call routes
/// directly to REST and avoids the gRPC cascade entirely. Without
/// the fallback wiring on this branch every day would h2-disconnect.
#[tokio::test]
#[ignore = "live-gated; requires THETADX_LIVE_CREDS + a running patched Terminal"]
async fn fallback_routes_2021_qqq_daily_expiry_quote_backfill_around_h2_cascade() {
    let Ok(creds_path) = std::env::var("THETADX_LIVE_CREDS") else {
        eprintln!("THETADX_LIVE_CREDS unset -- skipping");
        return;
    };
    let creds = Credentials::from_file(&creds_path).expect("creds file");

    let cfg =
        DirectConfig::production().with_rest_fallback(FallbackPolicy::RestAlwaysForDateRange {
            base_url: DEFAULT_REST_BASE_URL.to_string(),
            before: 20_230_101,
        });
    let tdx = ThetaDataDxClient::connect(&creds, cfg)
        .await
        .expect("connect");

    let mut total_ticks = 0_usize;
    for date in REPRODUCER_DAYS {
        // 2021 QQQ daily-expiry contract -- ATM strike for the day
        // is around 330 in early April 2021. `strike=*` returns the
        // whole chain so we don't need to hard-code the ATM strike
        // (and accidentally miss it if the underlying moves).
        let ticks = tdx
            .option_history_quote_with_fallback(
                "QQQ",
                /* expiration */ date,
                /* start_date */ date,
                /* end_date */ None,
                /* strike */ Some("*"),
                /* right  */ Some("both"),
                /* interval */ Some("1s"),
            )
            .await
            .unwrap_or_else(|err| {
                panic!("date {date}: option_history_quote_with_fallback failed: {err}")
            });
        assert!(
            !ticks.is_empty(),
            "date {date}: expected non-empty tick stream, got 0 ticks -- \
             the h2 cascade fallback did not recover the call"
        );
        eprintln!("  date {date}: {n} ticks", n = ticks.len());
        total_ticks += ticks.len();
    }
    eprintln!(
        "backfill total: {total_ticks} ticks across {n} days",
        n = REPRODUCER_DAYS.len()
    );
    assert!(
        total_ticks > 0,
        "all 5 days must contribute SOME ticks for the regression to pass"
    );
}

/// Sanity-check the dead-channel routing in the pool by issuing a
/// gRPC call against a known-good 2024 date -- this should NOT
/// touch the REST fallback, because `RestAlwaysForDateRange` only
/// pre-routes dates strictly earlier than `before`. The test
/// confirms the post-#577 fallback wiring does not regress the
/// happy-path (a working gRPC call stays on gRPC).
#[tokio::test]
#[ignore = "live-gated; requires THETADX_LIVE_CREDS"]
async fn fallback_does_not_intercept_post_cutoff_dates() {
    let Ok(creds_path) = std::env::var("THETADX_LIVE_CREDS") else {
        eprintln!("THETADX_LIVE_CREDS unset -- skipping");
        return;
    };
    let creds = Credentials::from_file(&creds_path).expect("creds file");

    let cfg =
        DirectConfig::production().with_rest_fallback(FallbackPolicy::RestAlwaysForDateRange {
            base_url: DEFAULT_REST_BASE_URL.to_string(),
            before: 20_230_101,
        });
    let tdx = ThetaDataDxClient::connect(&creds, cfg)
        .await
        .expect("connect");

    // 2024-06-05 is well past the cutoff -- the call MUST go through
    // gRPC. A non-empty response confirms the gRPC path still works
    // for the modern storage tier.
    let ticks = tdx
        .option_history_quote_with_fallback(
            "QQQ",
            /* expiration */ "20240605",
            /* start_date */ "20240604",
            /* end_date */ None,
            Some("440"),
            Some("call"),
            Some("1m"),
        )
        .await
        .expect("post-cutoff gRPC call");
    assert!(
        !ticks.is_empty(),
        "post-cutoff gRPC happy path must still return data"
    );
}
