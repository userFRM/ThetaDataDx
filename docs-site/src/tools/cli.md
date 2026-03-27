# CLI (tdx)

Command-line interface for querying ThetaData market data. All 61 endpoints plus offline Greeks.

## Installation

```bash
cargo install --path crates/thetadatadx-cli
```

## Setup

Create a `creds.txt` with your ThetaData email (line 1) and password (line 2).

## Global Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--creds <path>` | `creds.txt` | Credentials file |
| `--config <preset>` | `production` | `production` or `dev` |
| `--format <fmt>` | `table` | `table`, `json`, or `csv` |

## Quick Examples

```bash
# Stock EOD data
tdx stock history_eod AAPL 20240101 20240301

# As JSON
tdx stock history_eod AAPL 20240101 20240301 --format json

# Option chain
tdx option list_expirations SPY
tdx option list_strikes SPY 20240419

# Option EOD with Greeks
tdx option history_greeks_eod SPY 20240419 500000 C 20240101 20240301

# Snapshot quotes
tdx stock snapshot_quote AAPL,MSFT,GOOGL

# Offline Greeks (no server needed)
tdx greeks 450 455 0.05 0.015 0.082 8.5 call
tdx iv 450 455 0.05 0.015 0.082 8.5 call
```

## Endpoint Coverage

| Category | Count |
|----------|-------|
| Stock | 14 subcommands |
| Option | 34 subcommands |
| Index | 9 subcommands |
| Rate | 1 subcommand |
| Calendar | 3 subcommands |
| Offline | 2 subcommands (greeks, iv) |

## Scripting

```bash
# Export to CSV
tdx stock history_eod AAPL 20240101 20240301 --format csv > aapl_eod.csv

# Scan multiple symbols
for symbol in AAPL MSFT GOOGL AMZN META; do
    tdx stock snapshot_quote "$symbol" --format json
done

# Get nearest expiration
EXP=$(tdx option list_expirations SPY --format json | jq -r '.[0]')
tdx option list_strikes SPY "$EXP"
```
