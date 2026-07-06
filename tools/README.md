<p align="center">
  <img src="../assets/logo.svg" alt="ThetaDataDx" width="100%" />
</p>

# Tools

Standalone applications built on the `thetadatadx` SDK. Each tool has its own README with full setup and usage details.

| Tool | Path | Description |
|------|------|-------------|
| MCP server | [`tools/mcp/`](mcp/README.md) | Model Context Protocol server (`thetadatadx-mcp`). Exposes every generated market-data endpoint plus one offline tool (`ping`) as JSON-RPC 2.0 tool calls over stdio. |
| REST / WebSocket server | [`tools/server/`](server/README.md) | Drop-in `thetadatadx-server` for the ThetaData JVM terminal. Listens on port 25503, serves the same `/v3/*` REST routes and WebSocket streaming surface. |

Both tools read email/password credentials from a `creds.txt` file (`--creds <path>`, default `creds.txt`) and share the same credential precedence: a ThetaData API key via the `--api-key` flag or the `THETADATA_API_KEY` environment variable takes priority, otherwise email/password from the `THETADATA_EMAIL` / `THETADATA_PASSWORD` environment variables, otherwise the `creds.txt` file. MCP and server accept an offline mode when credentials are absent.
