# Notebooks

Interactive Jupyter notebooks demonstrating the `thetadatadx` Python SDK. Each notebook is self-contained and progresses from basics to advanced real-time streaming.

## Prerequisites

```bash
pip install thetadatadx[all] jupyter matplotlib
```

The `[all]` extra installs both `pandas` and `polars` support. If you only need one:

```bash
pip install thetadatadx[pandas] jupyter matplotlib
```

Notebook 106 additionally requires `ipywidgets`:

```bash
pip install ipywidgets
```

## Credentials

All notebooks expect ThetaData credentials. Create a `creds.txt` file in this directory (or wherever you run Jupyter from) with your ThetaData email on line 1 and password on line 2:

```
your-email@example.com
your-password
```

This file is loaded by `Credentials.from_file("creds.txt")` in each notebook. Alternatively, you can pass credentials inline:

```python
from thetadatadx import Credentials
creds = Credentials("your-email@example.com", "your-password")
```

**Do not commit `creds.txt` to version control.** It is already in `.gitignore`.

## Notebooks

| # | Notebook | Description |
|---|----------|-------------|
| 101 | [Getting Started](101_getting_started.ipynb) | Connect, list symbols, EOD data, OHLC bars, option expirations/strikes, Greeks calculator |
| 102 | [Historical Analysis](102_historical_analysis.ipynb) | Price charts, daily returns distribution, intraday volume, bid-ask spreads, multi-symbol comparison |
| 103 | [Options Chain](103_options_chain.ipynb) | Build a full option chain DataFrame, implied volatility smile, open interest visualization |
| 104 | [Greeks Surface](104_greeks_surface.ipynb) | 3D volatility surface, delta/gamma/theta heatmaps, time decay analysis |
| 105 | [Real-Time Streaming](105_realtime_streaming.ipynb) | FPSS quote and trade subscriptions, event collection, trade flow summary |
| 106 | [Live Option Chain](106_live_option_chain.ipynb) | In-notebook interactive chain with ipywidgets, auto-refresh, IV smile and term structure plots |
| 107 | [Full Trade Stream](107_full_trade_stream.ipynb) | Stock + option trade firehose, block detector, momentum analysis, premium flow dashboard |

## Notebook Details

### 101 - Getting Started

The entry point. Covers installation, authentication, and a tour of the core API surface:

1. Install `thetadatadx` with pandas support
2. Create `Credentials` and connect a `ThetaDataDx` client to production servers
3. List all available stock symbols
4. Fetch AAPL end-of-day data and convert to a pandas DataFrame
5. Fetch 1-minute OHLC bars for a single trading day
6. List SPY option expirations and strikes
7. Compute all 22 Greeks (including IV) with the built-in Rust Black-Scholes calculator

### 102 - Historical Analysis

Quantitative analysis patterns using historical data:

1. Full-year AAPL EOD price chart
2. Log return distribution with histogram
3. Intraday trade volume distribution across the trading session
4. NBBO quote data: bid-ask spread analysis over the day
5. Multi-symbol comparison (AAPL, MSFT, GOOGL)

### 103 - Options Chain

Build a complete option chain from scratch:

1. List all expirations for SPY
2. Identify the nearest monthly expiration (third Friday, 14+ DTE)
3. Fetch all strikes for that expiration
4. Pull snapshot NBBO quotes for every call and put
5. Assemble the unified option chain DataFrame
6. Compute and plot the implied volatility smile
7. Visualize open interest across strikes

### 104 - Greeks Surface

Three-dimensional visualization of the options Greeks:

1. Fetch 6 expirations spanning near-term to several months out
2. Compute full Greeks across all expirations and strikes
3. Build the IV surface (strike x expiration x IV)
4. 3D surface plot of implied volatility
5. Delta, gamma, theta heatmaps across the surface
6. Time decay analysis: theta acceleration as expiration approaches

### 105 - Real-Time Streaming

Introduction to the FPSS (Fast Push Streaming Service) client:

1. Start streaming via `ThetaDataDx` (persistent TCP connection)
2. Subscribe to AAPL quote updates
3. Collect quote events over a 10-second window
4. Display quote updates in a table
5. Subscribe to AAPL trades
6. Trade flow summary
7. Clean unsubscribe and shutdown

**Note:** FPSS streaming requires real-time data access in your ThetaData subscription. Data is only available during market hours (9:30 AM - 4:00 PM ET).

### 106 - Live Option Chain

A fully interactive, live-updating option chain rendered inside the notebook:

- Dropdown expiration selector
- Full chain with Greeks computed via the Rust Black-Scholes calculator
- ITM/ATM color coding
- One-click and auto-refresh modes
- IV smile plot and multi-expiration term structure visualization

Requires `ipywidgets`. For a richer standalone GUI with tabbed expirations and configurable display, see the [Streamlit live-chain app](../tools/live-chain/).

### 107 - Full Trade Stream

Advanced real-time trade analysis covering the full firehose:

1. Connect and authenticate
2. Subscribe to stock trade streams for a watchlist of symbols
3. Real-time analysis: volume by symbol, large block detection, price momentum, buy/sell imbalance
4. Subscribe to option trades per-contract (underlying, expiration, strike, right)
5. Combined stock + option dashboard with live-updating summary

The Rust core decodes FIT payloads and delta-decompresses ticks before they reach Python. Each event dict has named fields (`price`, `size`, `exchange`, `condition`, `ms_of_day`, etc.) - no raw payload handling required.

## Running

Start Jupyter from the repository root or this directory:

```bash
cd notebooks
jupyter notebook
```

Or with JupyterLab:

```bash
jupyter lab
```

Open notebook 101 first and work through them in order. Each notebook links to the next at the bottom.
