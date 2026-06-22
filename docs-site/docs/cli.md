---
title: CLI
description: Query any endpoint from the command line with the thetadatadx binary.
---

# CLI

`thetadatadx` queries every [reference endpoint](/reference/) from the command line, plus the offline Greeks tools. Subcommands are generated from the same endpoint registry as the SDKs, so the CLI never lags the library.

## Install and set up

```bash
cargo install thetadatadx-cli --git https://github.com/userFRM/ThetaDataDx
```

Sign in any of these ways: pass `--api-key <KEY>`, set `THETADATA_API_KEY` in the environment, set `THETADATA_EMAIL` + `THETADATA_PASSWORD` in the environment, or create `creds.txt` (email line 1, password line 2) in your working directory and point at it with `--creds`. They resolve in that order, highest first. These are the same names the SDK, the server, and the [MCP server](/mcp) read, so one login authenticates every tool.

## Usage

```bash
# Stock EOD across a range
thetadatadx stock history_eod AAPL 20250303 20250306

# Discover an option chain
thetadatadx option list_expirations SPY
thetadatadx option list_strikes SPY 20250321

# EOD Greeks for one pinned contract
thetadatadx option history_greeks_eod SPY 20250321 20250303 20250306 570 C

# Snapshot quotes for several symbols
thetadatadx stock snapshot_quote AAPL,MSFT,GOOGL

# Offline Black-Scholes (no credentials needed)
thetadatadx greeks 450 455 0.05 0.015 0.082 8.5 call
thetadatadx iv 450 455 0.05 0.015 0.082 8.5 call
```

Commands mirror the endpoint names, and every parameter — required and optional — is positional, in the order the matching [reference page](/reference/) lists them. `thetadatadx <category> <endpoint> --help` prints the exact shape, defaults included.

## Global flags

| Flag | Default | Description |
|---|---|---|
| `--api-key <key>` | — | Authenticate with a ThetaData API key (or set `THETADATA_API_KEY`). Takes precedence over the environment variables and the email/password path. |
| `--creds <path>` | `creds.txt` | Credentials file (email line 1, password line 2). Used when no API key and no `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair is set. |
| `--config <preset>` | `production` | `production` or `dev`. |
| `--format <fmt>` | `table` | `table`, `json`, `json-raw`, or `csv`. `json-raw` emits dates as raw `YYYYMMDD` integers (and `ms_of_day` as raw milliseconds) instead of the ISO-formatted values `json` produces. |

## Scripting

```bash
# Export to CSV
thetadatadx stock history_eod AAPL 20250303 20250306 --format csv > aapl_eod.csv

# Chain into jq
EXP=$(thetadatadx option list_expirations SPY --format json | jq -r '.[0]')
thetadatadx option list_strikes SPY "$EXP"
```

Use `--format csv` for spreadsheets and pipelines, `--format json` for `jq`.
