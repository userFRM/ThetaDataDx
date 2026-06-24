# thetadatadx — ThetaDataDx CLI

Command-line interface for querying ThetaData market data.

> **FLATFILES coverage:** the `thetadatadx flatfile` subcommand group exposes the FLATFILES whole-universe daily-blob surface alongside historical (request/response) and streaming. The distribution serves option `trade_quote` / `open_interest` / `eod` and stock `trade_quote` / `eod`. Example: `thetadatadx flatfile trade_quote 20260428 --format csv -o spy_trade_quotes.csv`.

## Install

```bash
cargo install --path tools/cli
```

Or build from the workspace root:

```bash
cargo build --release -p thetadatadx-cli
# binary at target/release/thetadatadx
```

## Setup

Sign in any of these ways, resolved in this order (highest first): pass `--api-key <KEY>`, set `THETADATA_API_KEY` in the environment, set `THETADATA_EMAIL` + `THETADATA_PASSWORD` in the environment, or create a `creds.txt` file and point at it with `--creds`. These are the same names the SDK, the server, and the MCP server read, so one login authenticates every tool.

```bash
# API key on the flag or in the environment
thetadatadx --api-key YOUR_API_KEY auth
export THETADATA_API_KEY="YOUR_API_KEY"

# Email + password in the environment
export THETADATA_EMAIL="your-email@example.com"
export THETADATA_PASSWORD="your-password"
```

Or a `creds.txt` file (email line 1, password line 2):

```
your-email@example.com
your-password
```

## Usage

```bash
# Test authentication
thetadatadx auth --creds creds.txt

# Stock data
thetadatadx stock list_symbols
thetadatadx stock list_dates EOD AAPL
thetadatadx stock history_eod AAPL 20240101 20240301
thetadatadx stock history_ohlc AAPL 20240315 1m              # 1-min bars
thetadatadx stock history_ohlc_range AAPL 20240101 20240301 1m
thetadatadx stock history_trade AAPL 20240315
thetadatadx stock history_quote AAPL 20240315 1m
thetadatadx stock history_trade_quote AAPL 20240315
thetadatadx stock snapshot_ohlc AAPL,MSFT,GOOGL
thetadatadx stock snapshot_trade AAPL,MSFT,GOOGL
thetadatadx stock snapshot_quote AAPL,MSFT,GOOGL
thetadatadx stock snapshot_market_value AAPL,MSFT
thetadatadx stock at_time_trade AAPL 20240101 20240301 09:30:00.000   # 9:30 AM ET
thetadatadx stock at_time_quote AAPL 20240101 20240301 09:30:00.000

# Options
thetadatadx option list_symbols
thetadatadx option list_expirations SPY
thetadatadx option list_strikes SPY 20240419
thetadatadx option list_dates EOD SPY 20240419 500 C
thetadatadx option list_contracts EOD SPY 20240315
thetadatadx option history_trade SPY 20240419 500 C 20240315
thetadatadx option history_quote SPY 20240419 500 C 20240315 1m
thetadatadx option history_eod SPY 20240419 500 C 20240101 20240301
thetadatadx option history_ohlc SPY 20240419 500 C 20240315 1m
thetadatadx option history_trade_quote SPY 20240419 500 C 20240315
thetadatadx option history_open_interest SPY 20240419 500 C 20240315

# Option snapshots
thetadatadx option snapshot_ohlc SPY 20240419 500 C
thetadatadx option snapshot_trade SPY 20240419 500 C
thetadatadx option snapshot_quote SPY 20240419 500 C
thetadatadx option snapshot_open_interest SPY 20240419 500 C
thetadatadx option snapshot_market_value SPY 20240419 500 C
thetadatadx option snapshot_greeks_implied_volatility SPY 20240419 500 C
thetadatadx option snapshot_greeks_all SPY 20240419 500 C
thetadatadx option snapshot_greeks_first_order SPY 20240419 500 C
thetadatadx option snapshot_greeks_second_order SPY 20240419 500 C
thetadatadx option snapshot_greeks_third_order SPY 20240419 500 C

# Option Greeks history
thetadatadx option history_greeks_eod SPY 20240419 500 C 20240101 20240301
thetadatadx option history_greeks_all SPY 20240419 500 C 20240315 1m
thetadatadx option history_trade_greeks_all SPY 20240419 500 C 20240315
thetadatadx option history_greeks_first_order SPY 20240419 500 C 20240315 1m
thetadatadx option history_trade_greeks_first_order SPY 20240419 500 C 20240315
thetadatadx option history_greeks_second_order SPY 20240419 500 C 20240315 1m
thetadatadx option history_trade_greeks_second_order SPY 20240419 500 C 20240315
thetadatadx option history_greeks_third_order SPY 20240419 500 C 20240315 1m
thetadatadx option history_trade_greeks_third_order SPY 20240419 500 C 20240315
thetadatadx option history_greeks_implied_volatility SPY 20240419 500 C 20240315 1m
thetadatadx option history_trade_greeks_implied_volatility SPY 20240419 500 C 20240315

# Option at-time queries
thetadatadx option at_time_trade SPY 20240419 500 C 20240101 20240301 09:30:00.000
thetadatadx option at_time_quote SPY 20240419 500 C 20240101 20240301 09:30:00.000

# Indices
thetadatadx index list_symbols
thetadatadx index list_dates SPX
thetadatadx index history_eod SPX 20240101 20240301
thetadatadx index history_ohlc SPX 20240101 20240301 1m
thetadatadx index history_price SPX 20240315 1m
thetadatadx index snapshot_ohlc SPX,NDX,RUT
thetadatadx index snapshot_price SPX,NDX,RUT
thetadatadx index snapshot_market_value SPX,NDX,RUT
thetadatadx index at_time_price SPX 20240101 20240301 09:30:00.000

# Interest rates
thetadatadx rate history_eod SOFR 20240101 20240301

# Market calendar
thetadatadx calendar open_today
thetadatadx calendar year 2024
thetadatadx calendar on_date 20240315

# Black-Scholes Greeks (offline, no server needed)
thetadatadx greeks 450 450 0.05 0.015 0.082 8.5 call
thetadatadx iv 450 450 0.05 0.015 0.082 8.5 call
```

## Output formats

```bash
thetadatadx stock history_eod AAPL 20240101 20240301                  # pretty table (default)
thetadatadx stock history_eod AAPL 20240101 20240301 --format json     # JSON array
thetadatadx stock history_eod AAPL 20240101 20240301 --format json-raw # JSON with raw YYYYMMDD integer dates
thetadatadx stock history_eod AAPL 20240101 20240301 --format csv      # CSV
```

## Global flags

| Flag | Default | Description |
|------|---------|-------------|
| `--api-key <key>` | | Authenticate with a ThetaData API key (or set `THETADATA_API_KEY`). Takes precedence over the environment variables and the email/password path |
| `--creds <path>` | `creds.txt` | Credentials file (email line 1, password line 2). Used when no API key and no `THETADATA_EMAIL` + `THETADATA_PASSWORD` pair is set |
| `--config <preset>` | `production` | `production` or `dev` |
| `--format <fmt>` | `table` | `table`, `json`, `json-raw`, or `csv`; `json-raw` emits dates as raw `YYYYMMDD` integers instead of the ISO values `json` produces |
| `--timeout-ms <ms>` | | Per-call deadline in milliseconds; on expiry the in-flight request is cancelled |

## Endpoint coverage

The 61 ThetaDataDx endpoints are organized by category (Stock + Option + Index + Rate + Calendar = 14 + 34 + 9 + 1 + 3 = 61). Two additional offline commands (`greeks`, `iv`) are not ThetaData endpoints — they call the in-process Black-Scholes calculator:

| Category | Count | Subcommands |
|----------|-------|-------------|
| Stock | 14 | `list_symbols`, `list_dates`, `history_eod`, `history_ohlc`, `history_ohlc_range`, `history_trade`, `history_quote`, `history_trade_quote`, `snapshot_ohlc`, `snapshot_trade`, `snapshot_quote`, `snapshot_market_value`, `at_time_trade`, `at_time_quote` |
| Option | 34 | `list_symbols`, `list_dates`, `list_expirations`, `list_strikes`, `list_contracts`, `snapshot_ohlc`, `snapshot_trade`, `snapshot_quote`, `snapshot_open_interest`, `snapshot_market_value`, `snapshot_greeks_implied_volatility`, `snapshot_greeks_all`, `snapshot_greeks_first_order`, `snapshot_greeks_second_order`, `snapshot_greeks_third_order`, `history_eod`, `history_ohlc`, `history_trade`, `history_quote`, `history_trade_quote`, `history_open_interest`, `history_greeks_eod`, `history_greeks_all`, `history_trade_greeks_all`, `history_greeks_first_order`, `history_trade_greeks_first_order`, `history_greeks_second_order`, `history_trade_greeks_second_order`, `history_greeks_third_order`, `history_trade_greeks_third_order`, `history_greeks_implied_volatility`, `history_trade_greeks_implied_volatility`, `at_time_trade`, `at_time_quote` |
| Index | 9 | `list_symbols`, `list_dates`, `history_eod`, `history_ohlc`, `history_price`, `snapshot_ohlc`, `snapshot_price`, `snapshot_market_value`, `at_time_price` |
| Rate | 1 | `history_eod` |
| Calendar | 3 | `open_today`, `on_date`, `year` |
| Offline (not endpoints) | 2 | `greeks`, `iv` — local Black-Scholes calculator, maps to MCP tools `all_greeks` and `implied_volatility` respectively |
