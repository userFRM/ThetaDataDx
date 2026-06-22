// Smoke test: the `thetadatadx flatfile` subcommand surface parses correctly
// and the `--help` output advertises every per-(sec_type, req_type)
// shortcut plus the generic `request` arm. No live server reach.
//
//  / issue #433 acceptance criterion.

use std::process::Command;

fn binary() -> std::path::PathBuf {
    // CARGO_BIN_EXE_<name> points at the compiled `thetadatadx` binary at the
    // location cargo built it for the test runner. Falls back to the
    // typical workspace path when the env var is missing (e.g. when
    // someone runs the test via `cargo test -p thetadatadx-cli` from
    // a CI matrix that bypassed CARGO_BIN_EXE_*).
    std::env::var_os("CARGO_BIN_EXE_tdx")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.push("../../target/debug/thetadatadx");
            p
        })
}

#[test]
fn flatfile_help_lists_every_shortcut() {
    let out = Command::new(binary())
        .args(["flatfile", "--help"])
        .output()
        .expect("thetadatadx binary should exist (run `cargo build -p thetadatadx-cli` first)");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    for sub in [
        "trade_quote",
        "open_interest",
        "eod",
        "stock_trade_quote",
        "stock_eod",
        "request",
    ] {
        assert!(
            combined.contains(sub),
            "thetadatadx flatfile --help should advertise `{sub}`; got:\n{combined}"
        );
    }

    // The datasets the flat-file distribution does not serve must not be
    // advertised as convenience subcommands.
    for sub in ["quotes", "trades", "ohlc", "stock_quotes", "stock_trades"] {
        assert!(
            !combined.contains(&format!("  {sub} ")) && !combined.contains(&format!("  {sub}\n")),
            "thetadatadx flatfile --help must not advertise unserved subcommand `{sub}`; got:\n{combined}"
        );
    }
}

#[test]
fn flatfile_trade_quote_help_includes_format_flag() {
    let out = Command::new(binary())
        .args(["flatfile", "trade_quote", "--help"])
        .output()
        .expect("thetadatadx binary should exist");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("--format") && combined.contains("--output"),
        "thetadatadx flatfile trade_quote --help should advertise --format / --output; got:\n{combined}"
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
            "trade_quote",
            "--date",
            "20260428",
        ])
        .output()
        .expect("thetadatadx binary should exist");
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

#[test]
fn flatfile_request_rejects_unserved_req_and_sec_types() {
    // The generic `request` arm constrains both flags to the served matrix:
    // `index` is not a served security type, and `quote` / `trade` / `ohlc`
    // are not served as flat files. clap rejects each at parse time.
    for (flag, value) in [
        ("--sec-type", "index"),
        ("--req-type", "quote"),
        ("--req-type", "trade"),
        ("--req-type", "ohlc"),
    ] {
        let other = if flag == "--sec-type" {
            ["--req-type", "trade_quote"]
        } else {
            ["--sec-type", "option"]
        };
        let out = Command::new(binary())
            .args([
                "flatfile", "request", flag, value, other[0], other[1], "--date", "20260428",
            ])
            .output()
            .expect("thetadatadx binary should exist");
        assert!(
            !out.status.success(),
            "unserved {flag} {value} must fail at clap parse time"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(value) || stderr.contains("possible values"),
            "stderr should reject `{value}`; got: {stderr}"
        );
    }
}
