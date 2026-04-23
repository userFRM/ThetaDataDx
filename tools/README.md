# Tools

Standalone applications built on the `thetadatadx` SDK. Each tool has its own README with full setup and usage details.

| Tool | Path | Description |
|------|------|-------------|
| CLI | [`tools/cli/`](cli/README.md) | `tdx` command-line client. Exposes the 61 ThetaDataDx endpoints plus two offline calculator commands (`greeks`, `iv`). Output as `table`, `json`, or `csv`. |
| MCP server | [`tools/mcp/`](mcp/README.md) | Model Context Protocol server (`thetadatadx-mcp`). Exposes the 61 endpoints plus three offline tools (`ping`, `all_greeks`, `implied_volatility`) as JSON-RPC 2.0 tool calls over stdio. |
| REST / WebSocket server | [`tools/server/`](server/README.md) | Drop-in `thetadatadx-server` for the ThetaData Java terminal. Listens on port 25503, serves the same `/v3/*` REST routes and WebSocket streaming surface. |

All three tools read credentials from the same places (`--creds <path>`, `creds.txt`, or `THETA_EMAIL` / `THETA_PASSWORD` environment variables). MCP and server additionally accept an offline mode when credentials are absent.
