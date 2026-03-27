# Options & Greeks (Python)

## Option Chain Workflow

### Step 1: Discover Expirations

```python
exps = client.option_list_expirations("SPY")
print(exps[:10])  # ["20240419", "20240517", ...]
```

### Step 2: Get Strikes

```python
strikes = client.option_list_strikes("SPY", "20240419")
print(f"{len(strikes)} strikes for 2024-04-19")
```

### Step 3: Fetch Chain Data

```python
for strike in strikes[:10]:
    call = client.option_snapshot_quote("SPY", "20240419", strike, "C")
    put = client.option_snapshot_quote("SPY", "20240419", strike, "P")
    if call and put:
        c, p = call[0], put[0]
        print(f"Strike {strike}: "
              f"Call bid={c['bid']:.2f} ask={c['ask']:.2f} | "
              f"Put bid={p['bid']:.2f} ask={p['ask']:.2f}")
```

### Step 4: Server-Computed Greeks

```python
# All Greeks snapshot for a contract
greeks = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C")

# Historical EOD Greeks
greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500000", "C",
                                               "20240101", "20240301")

# Greeks on each individual trade
trade_greeks = client.option_history_trade_greeks_all("SPY", "20240419", "500000", "C",
                                                       "20240315")
```

### Full Chain Scan with DataFrames

```python
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
tdx = ThetaDataDx(creds, Config.production())

# Get nearest expiration
exps = tdx.option_list_expirations("SPY")
exp = exps[0]

# Get all strikes
strikes = tdx.option_list_strikes("SPY", exp)

# Fetch EOD data for all calls
chain = []
for strike in strikes:
    eod = tdx.option_history_eod("SPY", exp, strike, "C",
                                    "20240301", "20240301")
    if eod:
        eod[0]["strike"] = strike
        chain.append(eod[0])

# Convert to DataFrame
from thetadatadx import to_dataframe
df = to_dataframe(chain)
print(df[["strike", "close", "volume"]].head(20))
```

---

## Local Greeks Calculator

Compute Greeks locally without any server call. Works offline, no ThetaData subscription needed.

### All 22 Greeks at Once

```python
from thetadatadx import all_greeks

g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05,
    div_yield=0.015, tte=30/365, option_price=8.50, is_call=True
)

print(f"IV:    {g['iv']:.4f}")
print(f"Delta: {g['delta']:.4f}")
print(f"Gamma: {g['gamma']:.6f}")
print(f"Theta: {g['theta']:.4f}")
print(f"Vega:  {g['vega']:.4f}")
print(f"Rho:   {g['rho']:.4f}")
```

The result is a dict with 22 keys: `value`, `delta`, `gamma`, `theta`, `vega`, `rho`, `iv`, `iv_error`, `vanna`, `charm`, `vomma`, `veta`, `speed`, `zomma`, `color`, `ultima`, `d1`, `d2`, `dual_delta`, `dual_gamma`, `epsilon`, `lambda`.

### Implied Volatility Only

```python
from thetadatadx import implied_volatility

iv, err = implied_volatility(450.0, 455.0, 0.05, 0.015, 30/365, 8.50, True)
print(f"IV: {iv:.4f}, Error: {err:.6f}")
```

### Greeks Reference

#### First Order

| Greek | Description |
|-------|-------------|
| `delta` | dV/dS (Call: 0 to 1, Put: -1 to 0) |
| `theta` | Time decay per day (divided by 365) |
| `vega` | Sensitivity to volatility |
| `rho` | Sensitivity to interest rate |
| `epsilon` | Sensitivity to dividend yield |
| `lambda` | Leverage ratio (delta * S / V) |

#### Second Order

| Greek | Description |
|-------|-------------|
| `gamma` | d2V/dS2 |
| `vanna` | d2V/dSdv |
| `charm` | d2V/dSdt (delta decay) |
| `vomma` | d2V/dv2 (vol-of-vol) |
| `veta` | d2V/dvdt |

#### Third Order

| Greek | Description |
|-------|-------------|
| `speed` | d3V/dS3 |
| `zomma` | d3V/dS2dv |
| `color` | d3V/dS2dt |
| `ultima` | d3V/dv3 (clamped to [-100, 100]) |

### Edge Cases

| Condition | Behavior |
|-----------|----------|
| `t = 0` | Returns `0.0` for most Greeks |
| `v = 0` | Returns `0.0` for most Greeks |
| Deep ITM/OTM | IV solver may return high `iv_error` |
