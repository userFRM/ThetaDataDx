# thetadatadx-mcp

MCP (Model Context Protocol) server for [ThetaDataDx](https://github.com/userFRM/ThetaDataDx) - gives any LLM instant access to ThetaData market data via structured tool calls over stdio JSON-RPC 2.0.

## Architecture

```
LLM (any MCP-compatible client)
    |  JSON-RPC 2.0 over stdio
    v
thetadatadx-mcp (long-running process)
    |  Single ThetaDataDx client, authenticated once at startup
    v
ThetaData servers (MDDS gRPC + FPSS TCP)
```

The server authenticates **once** at startup, keeps the `ThetaDataDx` client alive, and serves tool calls instantly with zero per-request auth overhead.

## Install

```bash
cargo install thetadatadx-mcp --git https://github.com/userFRM/ThetaDataDx
```

Or build from source:

```bash
cd tools/mcp
cargo build --release
# Binary at ../../target/release/thetadatadx-mcp
```

## Configuration

### Credentials

Provide ThetaData credentials via **environment variables** (preferred) or a **creds file**:

```bash
# Environment variables (preferred)
export THETA_EMAIL="you@example.com"
export THETA_PASSWORD="your-password"

# Or a creds.txt file (line 1: email, line 2: password)
thetadatadx-mcp --creds ~/creds.txt
```

If no credentials are provided, the server starts in **offline mode** - only `ping`, `all_greeks`, and `implied_volatility` tools are available.

### Stdio MCP clients (config file)

Most MCP clients read an `mcpServers` block from a project-local or user-level settings file. The shape is identical across clients; consult your client's docs for the exact file path.

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "env": {
        "THETA_EMAIL": "you@example.com",
        "THETA_PASSWORD": "your-password"
      }
    }
  }
}
```

Or with a creds file:

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "args": ["--creds", "/path/to/creds.txt"]
    }
  }
}
```

### Cursor

Add to Cursor MCP settings (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "thetadata": {
      "command": "thetadatadx-mcp",
      "env": {
        "THETA_EMAIL": "you@example.com",
        "THETA_PASSWORD": "your-password"
      }
    }
  }
}
```

### Any MCP-compatible client

The server speaks standard MCP over stdio:
- **stdin**: one JSON-RPC 2.0 request per line
- **stdout**: one JSON-RPC 2.0 response per line
- **stderr**: structured logs (configurable via `RUST_LOG` env var)

## Available Tools (64 total)

61 registry endpoints + 3 offline tools (ping, all_greeks, implied_volatility) = 64 total.

### Meta (1)
- `ping` - server status (works offline)

### Offline Greeks (2, no ThetaData account needed)
- `all_greeks` - compute all 22 Black-Scholes Greeks
- `implied_volatility` - IV solver via bisection

### Stock Data (14 tools)
- `stock_list_symbols`, `stock_list_dates`
- `stock_snapshot_ohlc`, `stock_snapshot_trade`, `stock_snapshot_quote`, `stock_snapshot_market_value`
- `stock_history_eod`, `stock_history_ohlc`, `stock_history_ohlc_range`, `stock_history_trade`, `stock_history_quote`, `stock_history_trade_quote`
- `stock_at_time_trade`, `stock_at_time_quote`

### Option Data (34 tools)
- `option_list_symbols`, `option_list_dates`, `option_list_expirations`, `option_list_strikes`, `option_list_contracts`
- `option_snapshot_ohlc`, `option_snapshot_trade`, `option_snapshot_quote`, `option_snapshot_open_interest`, `option_snapshot_market_value`
- `option_snapshot_greeks_implied_volatility`, `option_snapshot_greeks_all`, `option_snapshot_greeks_first_order`, `option_snapshot_greeks_second_order`, `option_snapshot_greeks_third_order`
- `option_history_eod`, `option_history_ohlc`, `option_history_trade`, `option_history_quote`, `option_history_trade_quote`, `option_history_open_interest`
- `option_history_greeks_eod`, `option_history_greeks_all`, `option_history_trade_greeks_all`
- `option_history_greeks_first_order`, `option_history_trade_greeks_first_order`
- `option_history_greeks_second_order`, `option_history_trade_greeks_second_order`
- `option_history_greeks_third_order`, `option_history_trade_greeks_third_order`
- `option_history_greeks_implied_volatility`, `option_history_trade_greeks_implied_volatility`
- `option_at_time_trade`, `option_at_time_quote`

### Wildcard Option Queries

For option tools, MCP uses `"0"` as the wildcard value for `strike` and `expiration`.

- Use a pinned strike like `"strike":"385"` when you want one contract.
- Use `"strike":"0"` when you want a bulk chain-style response with contract identification fields on each row.
- `strike_range` filters a wildcard bulk selection around spot / ATM. It does **not** fan out a pinned strike into neighboring strikes.

This matches the current Java terminal behavior. The v3 REST surface uses `*` for the same wildcard concept; the MCP server uses `"0"` because it follows the underlying SDK contract.

### Index Data (9 tools)
- `index_list_symbols`, `index_list_dates`
- `index_snapshot_ohlc`, `index_snapshot_price`, `index_snapshot_market_value`
- `index_history_eod`, `index_history_ohlc`, `index_history_price`
- `index_at_time_price`

### Calendar & Rates (4 tools)
- `calendar_open_today`, `calendar_on_date`, `calendar_year`
- `interest_rate_history_eod`

## Example Tool Calls

### List tools

```json
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
```

### Fetch AAPL end-of-day data

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"stock_history_eod","arguments":{"symbol":"AAPL","start_date":"20240101","end_date":"20240301"}}}
```

Response:
```json
{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"ticks\":[{\"date\":20240102,\"ms_of_day\":57600000,\"open\":187.15,\"high\":188.44,\"low\":183.89,\"close\":185.64,\"volume\":82488700,\"count\":1036575,\"bid\":185.63,\"ask\":185.65,\"bid_size\":1,\"ask_size\":3},...],\"count\":41}"}]}}
```

### Fetch bulk option Greeks around ATM

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"option_history_greeks_eod","arguments":{"symbol":"SPY","expiration":"20230120","strike":"0","right":"C","start_date":"20221219","end_date":"20221220","strike_range":5}}}
```

This returns a filtered bulk response across multiple strikes. If you change `strike` to `"385"`, the response is limited to that single contract.

### Compute Greeks offline

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"all_greeks","arguments":{"spot":150.0,"strike":155.0,"rate":0.05,"dividend_yield":0.01,"time_to_expiry":0.25,"option_price":5.50,"is_call":true}}}
```

Response:
```json
{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\"value\":5.50,\"iv\":0.234,\"delta\":0.456,...}"}]}}
```

### Check server status

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ping","arguments":{}}}
```

## Logging

Set `RUST_LOG` to control verbosity:

```bash
RUST_LOG=debug thetadatadx-mcp       # verbose
RUST_LOG=warn thetadatadx-mcp        # quiet
RUST_LOG=thetadatadx=debug thetadatadx-mcp  # just the library
```

All logs go to **stderr**, never stdout (which is reserved for JSON-RPC).

## License

GPL-3.0-or-later
