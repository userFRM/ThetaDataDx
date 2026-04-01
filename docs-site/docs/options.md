---
title: Options & Greeks
description: Option chain workflows and local Black-Scholes Greeks calculator with 22 Greeks, IV solver, and edge-case handling.
---

# Options & Greeks

ThetaDataDx provides two complementary approaches to options analytics:

1. **Server-computed Greeks** - query ThetaData's servers for pre-calculated Greeks via the standard endpoints
2. **Local Greeks calculator** - compute all 22 Black-Scholes Greeks offline with no server call and no subscription required

## Option Chain Workflow

### Step 1: Discover Expirations

::: code-group
```rust [Rust]
let exps = client.option_list_expirations("SPY").await?;
for exp in &exps {
    println!("Expiration: {}", exp);  // "20240419", "20240517", ...
}
```
```python [Python]
exps = client.option_list_expirations("SPY")
print(exps[:10])  # ["20240419", "20240517", ...]
```
:::

### Step 2: Get Strikes

::: code-group
```rust [Rust]
let strikes = client.option_list_strikes("SPY", "20240419").await?;
println!("{} strikes available", strikes.len());
// Strikes are scaled integers: "500000" = $500.00
```
```python [Python]
strikes = client.option_list_strikes("SPY", "20240419")
print(f"{len(strikes)} strikes for 2024-04-19")
```
:::

### Step 3: Fetch Chain Data

::: code-group
```rust [Rust]
for strike in &strikes {
    let call_quotes = client.option_snapshot_quote(
        "SPY", "20240419", strike, "C"
    ).await?;
    let put_quotes = client.option_snapshot_quote(
        "SPY", "20240419", strike, "P"
    ).await?;

    if let (Some(c), Some(p)) = (call_quotes.first(), put_quotes.first()) {
        println!("Strike {}: Call bid={} ask={} | Put bid={} ask={}",
            strike, c.bid_price(), c.ask_price(),
            p.bid_price(), p.ask_price());
    }
}
```
```python [Python]
for strike in strikes[:10]:
    call = client.option_snapshot_quote("SPY", "20240419", strike, "C")
    put = client.option_snapshot_quote("SPY", "20240419", strike, "P")
    if call and put:
        c, p = call[0], put[0]
        print(f"Strike {strike}: "
              f"Call bid={c['bid']:.2f} ask={c['ask']:.2f} | "
              f"Put bid={p['bid']:.2f} ask={p['ask']:.2f}")
```
:::

### Step 4: Server-Computed Greeks

::: code-group
```rust [Rust]
// All Greeks snapshot for a contract
let greeks = client.option_snapshot_greeks_all(
    "SPY", "20240419", "500000", "C"
).await?;

// Historical EOD Greeks over a date range
let greeks_eod = client.option_history_greeks_eod(
    "SPY", "20240419", "500000", "C", "20240101", "20240301"
).await?;

// Greeks on each individual trade
let trade_greeks = client.option_history_trade_greeks_all(
    "SPY", "20240419", "500000", "C", "20240315"
).await?;
```
```python [Python]
# All Greeks snapshot for a contract
greeks = client.option_snapshot_greeks_all("SPY", "20240419", "500000", "C")

# Historical EOD Greeks
greeks_eod = client.option_history_greeks_eod("SPY", "20240419", "500000", "C",
                                               "20240101", "20240301")

# Greeks on each individual trade
trade_greeks = client.option_history_trade_greeks_all("SPY", "20240419", "500000", "C",
                                                       "20240315")
```
:::

### Full Chain Scan with DataFrames (Python)

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

Compute Greeks locally without any server call using the built-in Black-Scholes calculator. Works offline, no ThetaData subscription needed.

### All 22 Greeks at Once

The most common usage: compute IV from the market price, then derive all 22 Greeks in a single call.

::: code-group
```rust [Rust]
use tdbe::greeks;

let result = greeks::all_greeks(
    450.0,            // spot price
    455.0,            // strike price
    0.05,             // risk-free rate
    0.015,            // dividend yield
    30.0 / 365.0,     // time to expiration (years)
    8.50,             // market option price
    true,             // is_call
);

println!("Implied Volatility: {:.4}", result.iv);
println!("Delta:    {:.4}", result.delta);
println!("Gamma:    {:.6}", result.gamma);
println!("Theta:    {:.4} (daily)", result.theta);
println!("Vega:     {:.4}", result.vega);
println!("Rho:      {:.4}", result.rho);
println!("Vanna:    {:.6}", result.vanna);
println!("Charm:    {:.6}", result.charm);
println!("Vomma:    {:.6}", result.vomma);
println!("Speed:    {:.8}", result.speed);
println!("Zomma:    {:.8}", result.zomma);
println!("Color:    {:.8}", result.color);
println!("Ultima:   {:.6}", result.ultima);
```
```python [Python]
from thetadatadx import all_greeks

g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05,
    div_yield=0.015, tte=30/365, price=8.50, is_call=True
)

print(f"IV:    {g['iv']:.4f}")
print(f"Delta: {g['delta']:.4f}")
print(f"Gamma: {g['gamma']:.6f}")
print(f"Theta: {g['theta']:.4f}")
print(f"Vega:  {g['vega']:.4f}")
print(f"Rho:   {g['rho']:.4f}")
```
:::

::: tip
The result contains 22 keys: `value`, `delta`, `gamma`, `theta`, `vega`, `rho`, `iv`, `iv_error`, `vanna`, `charm`, `vomma`, `veta`, `speed`, `zomma`, `color`, `ultima`, `d1`, `d2`, `dual_delta`, `dual_gamma`, `epsilon`, `lambda`.
:::

### Implied Volatility Only

::: code-group
```rust [Rust]
let (iv, error) = greeks::implied_volatility(
    450.0,   // spot
    455.0,   // strike
    0.05,    // rate
    0.015,   // dividend yield
    30.0 / 365.0, // time to expiry
    8.50,    // market price
    true,    // is_call
);
println!("IV: {:.4}, Error: {:.6}", iv, error);
```
```python [Python]
from thetadatadx import implied_volatility

iv, err = implied_volatility(450.0, 455.0, 0.05, 0.015, 30/365, 8.50, True)
print(f"IV: {iv:.4f}, Error: {err:.6f}")
```
:::

The solver uses bisection with up to 128 iterations. The `error` return is the relative difference `(theoretical - market) / market`.

### Individual Greeks (Rust)

For targeted computation when you only need one or two values:

```rust
use tdbe::greeks;

let s = 450.0;  // spot
let x = 455.0;  // strike
let v = 0.20;   // volatility (sigma)
let r = 0.05;   // rate
let q = 0.015;  // dividend yield
let t = 30.0 / 365.0;  // time (years)

// First order
let delta = greeks::delta(s, x, v, r, q, t, true);
let theta = greeks::theta(s, x, v, r, q, t, true);  // daily (divided by 365)
let vega  = greeks::vega(s, x, v, r, q, t);
let rho   = greeks::rho(s, x, v, r, q, t, true);

// Second order
let gamma = greeks::gamma(s, x, v, r, q, t);
let vanna = greeks::vanna(s, x, v, r, q, t);
let charm = greeks::charm(s, x, v, r, q, t, true);
let vomma = greeks::vomma(s, x, v, r, q, t);

// Third order
let speed  = greeks::speed(s, x, v, r, q, t);
let zomma  = greeks::zomma(s, x, v, r, q, t);
let color  = greeks::color(s, x, v, r, q, t);
let ultima = greeks::ultima(s, x, v, r, q, t);  // clamped to [-100, 100]
```

---

## Greeks Reference

### First Order

| Greek | Description | Notes |
|-------|-------------|-------|
| `delta` | dV/dS | Call: 0 to 1, Put: -1 to 0 |
| `theta` | Time decay per day | Divided by 365 |
| `vega` | Sensitivity to volatility | Same for calls and puts |
| `rho` | Sensitivity to interest rate | |
| `epsilon` | Sensitivity to dividend yield | |
| `lambda` | Leverage ratio (delta * S / V) | |

### Second Order

| Greek | Description |
|-------|-------------|
| `gamma` | d2V/dS2 (same for calls and puts) |
| `vanna` | d2V/dSdv |
| `charm` | d2V/dSdt (delta decay) |
| `vomma` | d2V/dv2 (vol-of-vol) |
| `veta` | d2V/dvdt |

### Third Order

| Greek | Description |
|-------|-------------|
| `speed` | d3V/dS3 |
| `zomma` | d3V/dS2dv |
| `color` | d3V/dS2dt |
| `ultima` | d3V/dv3 (clamped to [-100, 100]) |

### Auxiliary

| Greek | Description |
|-------|-------------|
| `d1` | Black-Scholes d1 intermediate |
| `d2` | Black-Scholes d2 intermediate |
| `dual_delta` | dV/dK (sensitivity to strike) |
| `dual_gamma` | d2V/dK2 |

## Edge Cases

| Condition | Behavior |
|-----------|----------|
| `t = 0` (at expiry) | Returns `0.0` for most Greeks; `value()` returns intrinsic value |
| `v = 0` (zero vol) | Returns `0.0` for most Greeks; `value()` returns intrinsic value |
| Deep ITM/OTM | IV solver may return high error; check `iv_error` field |
| `ultima` overflow | Clamped to [-100, 100] range |
