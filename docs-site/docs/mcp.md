---
title: MCP Server
description: Give any Model Context Protocol client live access to every market-data endpoint.
---

# MCP Server

`thetadatadx-mcp-server` is a Model Context Protocol server over stdio: any MCP-capable client (Claude Desktop, Cursor, and others) gets a tool per market-data endpoint, speaking JSON-RPC 2.0.

## Configure your client

Most MCP clients read an `mcpServers` block from a project-local or user-level settings file; the shape is the same across clients (for example `.cursor/mcp.json` in Cursor). Point the client at `npx`, which downloads and runs the server on demand, with no toolchain to install:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "npx",
      "args": ["-y", "thetadatadx-mcp-server"],
      "env": {
        "THETADATA_API_KEY": "your-api-key"
      }
    }
  }
}
```

`npx -y thetadatadx-mcp-server` fetches a prebuilt binary for your platform (Linux and macOS on x64 and arm64, Windows on x64) and runs it; nothing else to install. To authenticate with an email and password instead of an API key, swap the `env` block:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "npx",
      "args": ["-y", "thetadatadx-mcp-server"],
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
Keep credentials in environment variables or a secrets manager, not in config files committed to version control.
:::

## Tools

Every generated market-data endpoint plus `ping`. Tool names and parameters match the [reference pages](/reference/) one-to-one, so the model's tool list is the same surface you read here.

Once connected, the server advertises only the tools your subscription grants. A tool appears when its asset class (stock, options, indices, or interest-rate) is covered by your subscription; a class your plan omits contributes no tools. FREE-tier classes stay listed because FREE grants delayed data. `ping` and the trading calendar are offered to every account. Each market-data tool's description names the subscription it needs. Gating is per asset class; within a subscribed class, a call to an endpoint above your tier still returns the usual permission error.

The endpoint tools named `*_flat_file_*` return a flat file's rows like any other endpoint tool, under the 50,000-row limit below. A flat file cannot be narrowed, so one past the limit is refused as soon as its 50,001st row arrives, without downloading the rest. To obtain the whole file, each flat file also has an `<endpoint>_to_path` tool, advertised by class like its endpoint. It writes the file to disk as the terminal writes it, however many rows it has, and returns the written path. It takes the file's `date`, an optional `output_path` (a temp path when omitted) and an optional `format`: `csv` (the default), `json`, `json_legacy`, `ndjson` (or its alias `jsonl`) or `html`.

- `option_flat_file_eod_to_path`: option end-of-day flat file.
- `option_flat_file_trade_quote_to_path`: option trade-quote flat file.
- `option_flat_file_open_interest_to_path`: option open-interest flat file.
- `option_flat_file_quote_to_path`: option 1-minute quote flat file.
- `stock_flat_file_eod_to_path`: stock end-of-day flat file.
- `stock_flat_file_trade_quote_to_path`: stock trade-quote flat file.
- `stock_flat_file_quote_to_path`: stock 1-minute quote flat file.
- `index_flat_file_eod_to_path`: index end-of-day flat file.

The `entitlements` tool reports the subscription tier held for each asset class, which is what decides the rest of the tool list.

Without credentials, the server still starts and serves the offline tool (`ping`), which is useful for testing the integration. The market-data endpoints and the flat-file tools need a live connection.

## Option queries from a model

- Pin one contract with a concrete strike: `"strike":"385"`.
- Use `"strike":"*"` when you want a bulk chain-style response; rows then carry contract-identity fields.
- `strike_range` narrows a bulk selection around the money; it does not fan a pinned strike out to neighbors.
- One call returns at most 50,000 rows. A larger result is refused with its row count, so narrow it: a symbol, a single strike and right, a `strike_range` or `max_dte`, a coarser `interval`, or a `start_time` / `end_time` window. A full-day tick chain belongs in the SDK's streaming history builders or the flat-file tools.

## Troubleshooting

::: details The client lists no tools
Run `thetadatadx-mcp-server` by hand: the process must start silently and wait on stdin. Anything printed to stdout breaks the JSON-RPC channel. Logs go to stderr by design, so a corrupted stdout usually means a wrapper script is echoing.
:::

::: details Only `ping` appears
That is offline mode: credentials were missing or rejected, or the vendor was unreachable. Check `THETADATA_API_KEY`, or `THETADATA_EMAIL` / `THETADATA_PASSWORD`, in the client's `env` block, and run with `RUST_LOG=debug` to see which of the two it was.

The connect is not blocking: the server answers `initialize` immediately and connects in the background, because a client that waits on the handshake times out. `tools/list` and `tools/call` wait up to five seconds for that connect to settle before they answer, so a listing or a call made at startup sees the connected surface. If the vendor takes longer than that, the listing is the offline set, which is the honest answer while there is no connection; list again once the connection is up.
:::

::: details Calls fail with permission errors
The account's tier doesn't cover the endpoint. Check the tier badge on the matching [reference page](/reference/) against [Subscriptions](/articles/subscriptions).
:::

::: details Debug logging
`RUST_LOG=debug thetadatadx-mcp-server` (stderr only; stdout stays clean for the protocol).
:::

::: warning
LLM output varies run to run, so treat model-generated parameter choices and analysis as drafts to verify, per [Building with AI / LLMs](/articles/ai-llms).
:::
