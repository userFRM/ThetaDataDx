# Jupyter Notebooks

Seven interactive notebooks demonstrating the Python SDK. Each notebook is self-contained and can be run with a valid ThetaData subscription.

All notebooks are in the `notebooks/` directory of the repository.

## Notebooks

### 101 -- Getting Started

**[`101_getting_started.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/101_getting_started.ipynb)**

Authentication, connecting, your first EOD and OHLC queries. Covers `Credentials`, `Config`, `ThetaDataDx`, and basic DataFrame conversion.

### 102 -- Historical Analysis

**[`102_historical_analysis.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/102_historical_analysis.ipynb)**

Deep dive into historical data: EOD time series, intraday OHLC bars at various intervals, tick-level trade and quote data. Demonstrates `_df` convenience methods and data visualization.

### 103 -- Options Chain

**[`103_options_chain.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/103_options_chain.ipynb)**

Complete option chain workflow: listing expirations, fetching strikes, snapshot quotes for calls and puts, building a chain DataFrame.

### 104 -- Greeks Surface

**[`104_greeks_surface.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/104_greeks_surface.ipynb)**

Volatility surfaces and Greeks visualization. Uses `all_greeks()` to compute IV across strikes and expirations. 3D surface plots of delta, gamma, and implied volatility.

### 105 -- Real-Time Streaming

**[`105_realtime_streaming.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/105_realtime_streaming.ipynb)**

FPSS streaming in a notebook: connecting, subscribing to quotes and trades, processing events with `next_event()`, and building a live quote table.

### 106 -- Live Option Chain

**[`106_live_option_chain.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/106_live_option_chain.ipynb)**

Combines historical option data with real-time FPSS streaming to build a live-updating option chain display.

### 107 -- Full Trade Stream

**[`107_full_trade_stream.ipynb`](https://github.com/userFRM/ThetaDataDx/blob/main/notebooks/107_full_trade_stream.ipynb)**

Full trade stream processing: subscribing to all stock trades via `subscribe_full_trades`, aggregating volume, and detecting unusual trade activity.

## Running the Notebooks

```bash
# Install with notebook extras
pip install thetadatadx[all] jupyter matplotlib

# Clone the repo
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/notebooks

# Create a creds.txt in the notebooks directory
echo "your-email@example.com" > creds.txt
echo "your-password" >> creds.txt

# Launch Jupyter
jupyter notebook
```
