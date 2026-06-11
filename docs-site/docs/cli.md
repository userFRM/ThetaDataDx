---
title: CLI
description: Query any endpoint from the command line with the tdx binary.
---

# CLI

`tdx` queries every [reference endpoint](/reference/) from the command line, plus the offline Greeks tools. Subcommands are generated from the same endpoint registry as the SDKs, so the CLI never lags the library.

## Install and set up

```bash
cargo install thetadatadx-cli --git https://github.com/userFRM/ThetaDataDx
```

Create `creds.txt` (email line 1, password line 2) in your working directory, or point at one with `--creds`.

## Usage

```bash
# Stock EOD across a range
tdx stock history_eod AAPL 20250303 20250306

# Discover an option chain
tdx option list_expirations SPY
tdx option list_strikes SPY 20250321

# EOD Greeks for one pinned contract
tdx option history_greeks_eod SPY 20250321 570 C 20250303 20250306

# Snapshot quotes for several symbols
tdx stock snapshot_quote AAPL,MSFT,GOOGL

# Offline Black-Scholes (no credentials needed)
tdx greeks 450 455 0.05 0.015 0.082 8.5 call
tdx iv 450 455 0.05 0.015 0.082 8.5 call
```

Commands mirror the endpoint names, and every parameter — required and optional — is positional, in the order the matching [reference page](/reference/) lists them. `tdx <category> <endpoint> --help` prints the exact shape, defaults included.

## Global flags

| Flag | Default | Description |
|---|---|---|
| `--creds <path>` | `creds.txt` | Credentials file. |
| `--config <preset>` | `production` | `production` or `dev`. |
| `--format <fmt>` | `table` | `table`, `json`, or `csv`. |

## Scripting

```bash
# Export to CSV
tdx stock history_eod AAPL 20250303 20250306 --format csv > aapl_eod.csv

# Chain into jq
EXP=$(tdx option list_expirations SPY --format json | jq -r '.[0]')
tdx option list_strikes SPY "$EXP"
```

Use `--format csv` for spreadsheets and pipelines, `--format json` for `jq`.
