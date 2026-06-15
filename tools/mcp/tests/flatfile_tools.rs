// Smoke test: the MCP server's flatfile tools (issue #431) are
// declared in the JSON-RPC `tools/list` response. The test pulls the
// MCP binary, sends an `initialize` followed by `tools/list`, and
// asserts the every flatfile tool name is present.
//
// We can't easily call into the MCP main.rs internals from a separate
// integration test (the binary's modules are private), so we drive
// the binary as a child process the same way a real MCP client would.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

fn binary() -> std::path::PathBuf {
    std::env::var_os("CARGO_BIN_EXE_thetadatadx-mcp")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            p.push("../../target/debug/thetadatadx-mcp");
            p
        })
}

#[test]
fn tools_list_includes_flatfile_tools() {
    let bin = binary();
    if !bin.exists() {
        eprintln!(
            "SKIP: {} does not exist; run `cargo build -p thetadatadx-mcp` first",
            bin.display()
        );
        return;
    }

    // Launch the MCP server in offline mode (no THETA_EMAIL set so
    // it stays in offline mode and answers `tools/list` immediately).
    let mut child = Command::new(&bin)
        .env_remove("THETA_EMAIL")
        .env_remove("THETA_PASSWORD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("MCP server should spawn");

    let stdin = child.stdin.as_mut().expect("stdin");
    // Initialize handshake — required before tools/list per the MCP
    // 2025-11-25 spec which the server advertises.
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#;
    writeln!(stdin, "{init}").unwrap();
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
    writeln!(stdin, "{list}").unwrap();
    stdin.flush().unwrap();

    // Read up to two responses (initialize + tools/list). Cap the wait
    // at ~5s so a hanging server doesn't deadlock the test runner.
    let stdout = child.stdout.take().expect("stdout");
    let reader = BufReader::new(stdout);

    let mut tools_response: Option<String> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    for line in reader.lines() {
        if std::time::Instant::now() > deadline {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.contains("\"id\":2") {
            tools_response = Some(line);
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();

    let response = tools_response.expect("MCP server should respond to tools/list");

    for tool in [
        "thetadatadx_flatfile_request",
        "thetadatadx_flatfile_option_quote",
        "thetadatadx_flatfile_option_trade",
        "thetadatadx_flatfile_option_trade_quote",
        "thetadatadx_flatfile_option_ohlc",
        "thetadatadx_flatfile_option_open_interest",
        "thetadatadx_flatfile_option_eod",
        "thetadatadx_flatfile_stock_quote",
        "thetadatadx_flatfile_stock_trade",
        "thetadatadx_flatfile_stock_trade_quote",
        "thetadatadx_flatfile_stock_eod",
    ] {
        assert!(
            response.contains(tool),
            "tools/list response should advertise `{tool}`; got: {response}"
        );
    }
}
