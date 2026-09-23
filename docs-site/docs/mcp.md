---
title: MCP Server
description: Give any Model Context Protocol client live access to every market-data endpoint.
---

# MCP Server

`thetadatadx-mcp-server` is a Model Context Protocol server over stdio: any MCP-capable client (Claude Desktop, Cursor, and others) gets a tool per market-data endpoint, speaking JSON-RPC 2.0.

## Configure your client

Most MCP clients read an `mcpServers` block from a project-local or user-level settings file; the shape is the same across clients (for example `.cursor/mcp.json` in Cursor). Point the client at `npx`, which downloads and runs the server on demand — no toolchain to install:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "npx",
      "args": ["-y", "thetadatadx-mcp-server@next"],
      "env": {
        "THETADATA_API_KEY": "your-api-key"
      }
    }
  }
}
```

`npx -y thetadatadx-mcp-server@next` fetches a prebuilt binary for your platform (Linux and macOS on x64 and arm64, Windows on x64) and runs it; nothing else to install. To authenticate with an email and password instead of an API key, swap the `env` block:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "npx",
      "args": ["-y", "thetadatadx-mcp-server@next"],
      "env": {
        "THETADATA_EMAIL": "you@example.com",
        "THETADATA_PASSWORD": "your-password"
      }
    }
  }
}
```

The server resolves credentials in this order, highest first: the `--api-key` flag, then `THETADATA_API_KEY`, then `THETADATA_EMAIL` + `THETADATA_PASSWORD`, then a `--creds` file (email on line 1, password on line 2). The same names authenticate the SDK, the server, and every binding.

### Rust users: build from source

If you already have a Rust toolchain, install the binary directly and set `"command": "thetadatadx-mcp-server"` instead of the `npx` invocation above:

```bash
cargo install thetadatadx-mcp-server --git https://github.com/userFRM/ThetaDataDx
```

::: warning
Keep credentials in environment variables or a secrets manager — not in config files committed to version control.
:::

## Tools

Every generated market-data endpoint plus `ping`. Tool names and parameters match the [reference pages](/reference/) one-to-one, so the model's tool list is the same surface you read here.

Once connected, the server advertises only the tools your subscription grants. A tool appears when its asset class — stock, options, indices, or interest-rate — is covered by your subscription; a class your plan omits contributes no tools. FREE-tier classes stay listed because FREE grants delayed data. `ping` and the trading calendar are offered to every account. The flat-file tools follow the class in their name, and the generic flat-file request appears for any account holding a stock or option tier, since those are the only classes with flat files. Each market-data tool's description names the subscription it needs. Gating is per asset class; within a subscribed class, a call to an endpoint above your tier still returns the usual permission error.

When credentials are present the connected surface also carries the flat-file tools, advertised by class the way the endpoint tools are: an account with only a stock tier sees the stock ones and the generic request, not the option ones. Each pulls a whole-universe daily blob for a single date, writes it to disk as CSV or JSON Lines, and returns the written path:

- `thetadatadx_flatfile_request`: generic flat-file request for a served `(sec_type, req_type)` pair; an unserved pair is rejected with a typed invalid-parameter error.
- `thetadatadx_flatfile_option_trade_quote`: option trade-quote flat file.
- `thetadatadx_flatfile_option_open_interest`: option open-interest flat file.
- `thetadatadx_flatfile_option_eod`: option end-of-day flat file.
- `thetadatadx_flatfile_stock_trade_quote`: stock trade-quote flat file.
- `thetadatadx_flatfile_stock_eod`: stock end-of-day flat file.

The `entitlements` tool reports the subscription tier held for each asset class, which is what decides the rest of the tool list.

Without credentials, the server still starts and serves the offline tool (`ping`) — useful for testing the integration. The flat-file tools and the market-data endpoints need a live connection.

## Option queries from a model

- Pin one contract with a concrete strike: `"strike":"385"`.
- Use `"strike":"*"` when you want a bulk chain-style response; rows then carry contract-identity fields.
- `strike_range` narrows a bulk selection around the money; it does not fan a pinned strike out to neighbors.

## Troubleshooting

::: details The client lists no tools
Run `thetadatadx-mcp-server` by hand: the process must start silently and wait on stdin. Anything printed to stdout breaks the JSON-RPC channel — logs go to stderr by design, so a corrupted stdout usually means a wrapper script is echoing.
:::

::: details Only `ping` appears
That is offline mode: credentials were missing or rejected, or the vendor was unreachable. Check `THETADATA_API_KEY`, or `THETADATA_EMAIL` / `THETADATA_PASSWORD`, in the client's `env` block, and run with `RUST_LOG=debug` to see which of the two it was.

The connect is not blocking: the server answers `initialize` immediately and connects in the background, because a client that waits on the handshake times out. `tools/list` waits up to five seconds for that connect to settle before it answers, so a listing made at startup reports the connected surface. If the vendor takes longer than that, the listing is the offline set, which is the honest answer while there is no connection; list again once the connection is up.
:::

::: details Calls fail with permission errors
The account's tier doesn't cover the endpoint — check the tier badge on the matching [reference page](/reference/) against [Subscriptions](/articles/subscriptions).
:::

::: details Debug logging
`RUST_LOG=debug thetadatadx-mcp-server` (stderr only; stdout stays clean for the protocol).
:::

::: warning
LLM output varies run to run — treat model-generated parameter choices and analysis as drafts to verify, per [Building with AI / LLMs](/articles/ai-llms).
:::
