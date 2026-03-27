# Getting Started with Python

## Installation

```bash
pip install thetadatadx
```

With DataFrame support:

```bash
pip install thetadatadx[pandas]    # pandas DataFrames
pip install thetadatadx[polars]    # polars DataFrames
pip install thetadatadx[all]       # both
```

Requires Python 3.9+. Pre-built wheels are provided -- no Rust toolchain required.

### Building from Source

For unsupported platforms:

```bash
pip install maturin
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/python
maturin develop --release
```

## Credentials

Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2:

```text
your-email@example.com
your-password
```

## First Query

```python
from thetadatadx import Credentials, Config, DirectClient

# Authenticate and connect
creds = Credentials.from_file("creds.txt")
client = DirectClient(creds, Config.production())

# Fetch end-of-day stock data
eod = client.stock_history_eod("AAPL", "20240101", "20240301")
for tick in eod:
    print(f"{tick['date']}: O={tick['open']:.2f} H={tick['high']:.2f} "
          f"L={tick['low']:.2f} C={tick['close']:.2f} V={tick['volume']}")

# List option expirations
exps = client.option_list_expirations("SPY")
print(f"SPY expirations: {exps[:5]}")

# Compute Greeks (offline, no server call)
from thetadatadx import all_greeks

g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05,
    div_yield=0.015, tte=30/365, option_price=8.50, is_call=True
)
print(f"IV={g['iv']:.4f} Delta={g['delta']:.4f} Gamma={g['gamma']:.6f}")
```

## With pandas DataFrames

```python
from thetadatadx import Credentials, Config, DirectClient, to_dataframe

creds = Credentials.from_file("creds.txt")
client = DirectClient(creds, Config.production())

# Option 1: explicit conversion
eod = client.stock_history_eod("AAPL", "20240101", "20240301")
df = to_dataframe(eod)
print(df.head())

# Option 2: _df convenience methods
df = client.stock_history_eod_df("AAPL", "20240101", "20240301")
df = client.stock_history_ohlc_df("AAPL", "20240315", "60000")
```

Requires `pip install thetadatadx[pandas]`.

## Credentials from Environment Variables

```python
import os
from thetadatadx import Credentials

creds = Credentials(os.environ["THETA_EMAIL"], os.environ["THETA_PASS"])
```

## What's Next

- [Historical Data](historical.md) -- all 61 endpoints with Python examples
- [Real-Time Streaming](streaming.md) -- FPSS subscribe and next_event
- [Options & Greeks](options.md) -- option chain workflow and local Greeks
- [Jupyter Notebooks](notebooks.md) -- interactive examples
- [API Reference](api-reference.md) -- complete method and type listing
