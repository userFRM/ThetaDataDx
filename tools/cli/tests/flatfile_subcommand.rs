// Smoke test: the `tdx flatfile` subcommand surface parses correctly
// and the `--help` output advertises every per-(sec_type, req_type)
// shortcut plus the generic `request` arm. No live server reach.
//
//  / issue #433 acceptance criterion.

use std::process::Command;

fn binary() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> points at the compiled `tdx` binary at the
    // location cargo built it for the test runner. Falls back to the
    // typical workspace path when the env var is missing (e.g. when
    // someone runs the test via `cargo test -p thetadatadx-cli` from
    // a CI matrix that bypassed CARGO_BIN_EXE_*).
    std::env::var_os("CARGO_BIN_EXE_tdx")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.push("../../target/debug/tdx");
            p
        })
}

#[test]
fn flatfile_help_lists_every_shortcut() {
    let out = Command::new(binary())
        .args(["flatfile", "--help"])
        .output()
        .expect("tdx binary should exist (run `cargo build -p thetadatadx-cli` first)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    for sub in [
        "quotes",
        "trades",
        "trade_quote",
        "ohlc",
        "open_interest",
        "eod",
        "stock_quotes",
        "stock_trades",
        "stock_trade_quote",
        "stock_eod",
        "request",
    ] {
        assert!(
            combined.contains(sub),
            "tdx flatfile --help should advertise `{sub}`; got:\n{combined}"
        );
    }
}

#[test]
fn flatfile_quotes_help_includes_format_flag() {
    let out = Command::new(binary())
        .args(["flatfile", "quotes", "--help"])
        .output()
        .expect("tdx binary should exist");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("--format") && combined.contains("--output"),
        "tdx flatfile quotes --help should advertise --format / --output; got:\n{combined}"
    );
}

#[test]
fn flatfile_request_rejects_unknown_sec_type() {
    // No live network reach — clap rejects the bad enum at parse time.
    let out = Command::new(binary())
        .args([
            "flatfile",
            "request",
            "--sec-type",
            "bogus",
            "--req-type",
            "quote",
            "--date",
            "20260428",
        ])
        .output()
        .expect("tdx binary should exist");
    assert!(
        !out.status.success(),
        "unknown sec_type must fail at clap parse time"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bogus") || stderr.contains("possible values"),
        "stderr should mention the rejected sec-type; got: {stderr}"
    );
}
