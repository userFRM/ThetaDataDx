# tdx - ThetaDataDx CLI

Command-line interface for querying ThetaData market data.

## Install

```bash
cargo install --path tools/cli
```

Or build from the workspace root:

```bash
cargo build --release -p thetadatadx-cli
# binary at target/release/tdx
```

## Setup

Create a `creds.txt` file with your ThetaData credentials:

```
your-email@example.com
your-password
```

## Usage

```bash
# Test authentication
tdx auth --creds creds.txt

# Stock data
tdx stock list_symbols
tdx stock list_dates EOD AAPL
tdx stock history_eod AAPL 20240101 20240301
tdx stock history_ohlc AAPL 20240315 1m              # 1-min bars
tdx stock history_ohlc_range AAPL 20240101 20240301 1m
tdx stock history_trade AAPL 20240315
tdx stock history_quote AAPL 20240315 1m
tdx stock history_trade_quote AAPL 20240315
tdx stock snapshot_ohlc AAPL,MSFT,GOOGL
tdx stock snapshot_trade AAPL,MSFT,GOOGL
tdx stock snapshot_quote AAPL,MSFT,GOOGL
tdx stock snapshot_market_value AAPL,MSFT
tdx stock at_time_trade AAPL 20240101 20240301 09:30:00.000   # 9:30 AM ET
tdx stock at_time_quote AAPL 20240101 20240301 09:30:00.000

# Options
tdx option list_symbols
tdx option list_expirations SPY
tdx option list_strikes SPY 20240419
tdx option list_dates EOD SPY 20240419 500 C
tdx option list_contracts EOD SPY 20240315
tdx option history_trade SPY 20240419 500 C 20240315
tdx option history_quote SPY 20240419 500 C 20240315 1m
tdx option history_eod SPY 20240419 500 C 20240101 20240301
tdx option history_ohlc SPY 20240419 500 C 20240315 1m
tdx option history_trade_quote SPY 20240419 500 C 20240315
tdx option history_open_interest SPY 20240419 500 C 20240315

# Option snapshots
tdx option snapshot_ohlc SPY 20240419 500 C
tdx option snapshot_trade SPY 20240419 500 C
tdx option snapshot_quote SPY 20240419 500 C
tdx option snapshot_open_interest SPY 20240419 500 C
tdx option snapshot_market_value SPY 20240419 500 C
tdx option snapshot_greeks_implied_volatility SPY 20240419 500 C
tdx option snapshot_greeks_all SPY 20240419 500 C
tdx option snapshot_greeks_first_order SPY 20240419 500 C
tdx option snapshot_greeks_second_order SPY 20240419 500 C
tdx option snapshot_greeks_third_order SPY 20240419 500 C

# Option Greeks history
tdx option history_greeks_eod SPY 20240419 500 C 20240101 20240301
tdx option history_greeks_all SPY 20240419 500 C 20240315 1m
tdx option history_trade_greeks_all SPY 20240419 500 C 20240315
tdx option history_greeks_first_order SPY 20240419 500 C 20240315 1m
tdx option history_trade_greeks_first_order SPY 20240419 500 C 20240315
tdx option history_greeks_second_order SPY 20240419 500 C 20240315 1m
tdx option history_trade_greeks_second_order SPY 20240419 500 C 20240315
tdx option history_greeks_third_order SPY 20240419 500 C 20240315 1m
tdx option history_trade_greeks_third_order SPY 20240419 500 C 20240315
tdx option history_greeks_implied_volatility SPY 20240419 500 C 20240315 1m
tdx option history_trade_greeks_implied_volatility SPY 20240419 500 C 20240315

# Option at-time queries
tdx option at_time_trade SPY 20240419 500 C 20240101 20240301 09:30:00.000
tdx option at_time_quote SPY 20240419 500 C 20240101 20240301 09:30:00.000

# Indices
tdx index list_symbols
tdx index list_dates SPX
tdx index history_eod SPX 20240101 20240301
tdx index history_ohlc SPX 20240101 20240301 1m
tdx index history_price SPX 20240315 1m
tdx index snapshot_ohlc SPX,NDX,RUT
tdx index snapshot_price SPX,NDX,RUT
tdx index snapshot_market_value SPX,NDX,RUT
tdx index at_time_price SPX 20240101 20240301 09:30:00.000

# Interest rates
tdx rate history_eod SOFR 20240101 20240301

# Market calendar
tdx calendar open_today
tdx calendar year 2024
tdx calendar on_date 20240315

# Black-Scholes Greeks (offline, no server needed)
tdx greeks 450 450 0.05 0.015 0.082 8.5 call
tdx iv 450 450 0.05 0.015 0.082 8.5 call
```

## Output formats

```bash
tdx stock history_eod AAPL 20240101 20240301                  # pretty table (default)
tdx stock history_eod AAPL 20240101 20240301 --format json     # JSON array
tdx stock history_eod AAPL 20240101 20240301 --format csv      # CSV
```

## Global flags

| Flag | Default | Description |
|------|---------|-------------|
| `--creds <path>` | `creds.txt` | Credentials file |
| `--config <preset>` | `production` | `production` or `dev` |
| `--format <fmt>` | `table` | `table`, `json`, or `csv` |

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
