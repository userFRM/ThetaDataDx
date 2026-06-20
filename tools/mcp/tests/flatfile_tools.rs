// End-to-end check of the MCP server's offline `tools/list` gating. The
// server advertises only the tools `tools/call` can serve in the current
// connection state: with no credentials it stays in offline mode and must
// advertise exactly the three connection-free tools (`ping`, `all_greeks`,
// `implied_volatility`); the flat-file tools and registry historical
// endpoints require a live client and must be withheld until one connects.
//
// We can't easily call into the MCP main.rs internals from a separate
// integration test (the binary's modules are private), nor connect to a
// live upstream from CI, so the connected full-surface shape is asserted
// by the in-crate unit tests. Here we drive the binary as a child process
// the same way a real MCP client would and assert the offline shape.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Tools the offline server must advertise — and the only ones, since every
/// other tool requires a connected client. Mirrors `OFFLINE_TOOL_NAMES` in
/// the binary and the offline-mode tools the README / banner promise.
const OFFLINE_TOOLS: [&str; 3] = ["ping", "all_greeks", "implied_volatility"];

/// Connection-only tools that must NOT appear while the server is offline:
/// the flat-file tools and a representative registry historical endpoint.
const CONNECTION_ONLY_TOOLS: [&str; 7] = [
    "thetadatadx_flatfile_request",
    "thetadatadx_flatfile_option_trade_quote",
    "thetadatadx_flatfile_option_open_interest",
    "thetadatadx_flatfile_option_eod",
    "thetadatadx_flatfile_stock_trade_quote",
    "thetadatadx_flatfile_stock_eod",
    "option_history_greeks_eod",
];

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
fn offline_tools_list_advertises_only_offline_tools() {
    let bin = binary();
    if !bin.exists() {
        eprintln!(
            "SKIP: {} does not exist; run `cargo build -p thetadatadx-mcp` first",
            bin.display()
        );
        return;
    }

    // Launch the MCP server in offline mode (no THETA_EMAIL/THETA_PASSWORD so
    // it never connects and answers `tools/list` from the offline surface).
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

    // The three connection-free tools must be advertised offline.
    for tool in OFFLINE_TOOLS {
        let name_token = format!("\"name\":\"{tool}\"");
        assert!(
            response.contains(&name_token),
            "offline tools/list must advertise `{tool}`; got: {response}"
        );
    }

    // Every connection-only tool must be withheld while offline — `tools/call`
    // would reject them for lack of a client, so they must not be advertised.
    // Match the exact JSON name token so an unserved prefix can't accidentally
    // match a served suffix.
    for tool in CONNECTION_ONLY_TOOLS {
        let name_token = format!("\"name\":\"{tool}\"");
        assert!(
            !response.contains(&name_token),
            "offline tools/list must not advertise connection-only tool `{tool}`; got: {response}"
        );
    }
}
