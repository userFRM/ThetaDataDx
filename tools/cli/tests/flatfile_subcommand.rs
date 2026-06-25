// Smoke test: the `thetadatadx flatfile` subcommand surface parses correctly
// and the `--help` output advertises every per-(sec_type, req_type)
// shortcut plus the generic `request` arm. No live server reach.
//
//  / issue #433 acceptance criterion.

use std::process::Command;

fn binary() -> std::path::PathBuf {
    // Cargo always sets `CARGO_BIN_EXE_<bin-name>` for same-crate integration
    // tests, pointing at the binary it just built for the test runner. The bin
    // is named `thetadatadx`, so the var is `CARGO_BIN_EXE_thetadatadx`; cargo
    // honors `CARGO_TARGET_DIR` when computing the path, so this stays correct
    // under a relocated target directory (CI matrices).
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_thetadatadx"))
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
fn flatfile_request_rejects_unserved_req_types() {
    // The generic `request` arm constrains `--req-type` to the served matrix:
    // per-tick `quote` / `trade` / `ohlc` are not served as flat files, so clap
    // rejects each at parse time before any client work.
    for value in ["quote", "trade", "ohlc"] {
        let out = Command::new(binary())
            .args([
                "flatfile",
                "request",
                "--req-type",
                value,
                "--sec-type",
                "option",
                "--date",
                "20260428",
            ])
            .output()
            .expect("thetadatadx binary should exist");
        assert!(
            !out.status.success(),
            "unserved --req-type {value} must fail at clap parse time"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(value) || stderr.contains("possible values"),
            "stderr should reject `{value}`; got: {stderr}"
        );
    }
}

#[test]
fn flatfile_request_rejects_unserved_index_pair() {
    // `index` is a served security type (index EOD), so it parses; the served
    // matrix serves index EOD alone, so a non-EOD index pair such as
    // `index trade_quote` is rejected by the dispatch gate before any network
    // work, not at clap parse time.
    let out = Command::new(binary())
        .args([
            "flatfile",
            "request",
            "--sec-type",
            "index",
            "--req-type",
            "trade_quote",
            "--date",
            "20260428",
        ])
        .output()
        .expect("thetadatadx binary should exist");
    assert!(
        !out.status.success(),
        "unserved index trade_quote pair must fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("index"),
        "stderr should name the rejected index pair; got: {stderr}"
    );
}
