<p align="center">
  <img src="../../assets/logo.svg" alt="ThetaDataDx" width="100%" />
</p>

# ThetaDataDx Live Option Chain

Option chain viewer powered by the ThetaDataDx native SDK. No JVM
required --- connects directly to ThetaData servers via the `thetadatadx`
Python bindings. Quotes, last trades, open interest, and Greeks all come
from the snapshot endpoints, which refresh through the trading day.

## What it looks like

A full-width Streamlit dashboard with:

- **Sidebar**: credentials, ticker input, and display controls
- **Tabs**: one per expiration (nearest 12 shown), labeled with date and DTE
- **Option chain table**: calls on the left, strike in the center, puts on the right
- **Color coding**: ITM cells tinted green, ATM strike highlighted gold
- **Columns**: IV, Delta, Gamma, Theta, Bid, Ask, Last, Volume, OI
- **Auto-refresh**: configurable 2--30 second polling interval

## How to run

```bash
pip install -r requirements.txt
streamlit run app.py
```

Then open `http://localhost:8501` in your browser.

## Features

- Connects to ThetaData production servers via the native SDK (no JVM)
- Supports both inline credentials and `creds.txt` file authentication
- Spot price from `stock_snapshot_ohlc`
- Option quotes from `option_snapshot_quote` / `option_snapshot_trade`
- Open interest from `option_snapshot_open_interest`
- Greeks (IV, Delta, Gamma, Theta) from `option_snapshot_greeks_all`,
  computed server-side per contract
- Configurable number of strikes around ATM (5--50)
- Configurable auto-refresh interval
- Manual refresh button
- Works with any optionable ticker (SPY, AAPL, TSLA, QQQ, etc.)

## Requirements

- Python 3.12+
- A ThetaData account with an active market data subscription
- `thetadatadx` compiled for your platform (see main project README)
