---
title: Greeks Calculator
description: 22 local Black-Scholes Greeks and IV solver with no server round-trip. Offline, no subscription required.
---

# Greeks Calculator

ThetaDataDx ships a local Black-Scholes calculator in the Rust core (`tdbe/greeks.rs`) that computes 23 Greeks plus an IV solver without a server round-trip. No subscription is required for the calculator itself, which makes it usable for offline what-if analysis, batch scenario sweeps, and integration tests that must not depend on a live server. The server-computed Greeks endpoints are also exposed for callers that want the canonical upstream values.

This page covers the calculator at a first-use level. For the full reference — per-Greek formula mapping, wildcard chain workflows, edge cases — see [Options & Greeks](../options).

## All 23 Greeks from a market price

The most common call: feed in the market option price, back out IV, then derive every Greek in one pass.

::: code-group
```rust [Rust]
use thetadatadx::all_greeks;

let g = all_greeks(
    450.0,        // spot
    455.0,        // strike
    0.05,         // risk-free rate
    0.015,        // dividend yield
    30.0 / 365.0, // time to expiry (years)
    8.50,         // market option price
    "C",          // right ("C" / "P" or "call" / "put", case-insensitive)
);

println!("IV={:.4} delta={:.4} gamma={:.6} theta={:.4}/day",
    g.iv, g.delta, g.gamma, g.theta);
```
```python [Python]
from thetadatadx import all_greeks

g = all_greeks(
    spot=450.0, strike=455.0, rate=0.05, div_yield=0.015,
    tte=30/365, option_price=8.50, right="C",
)
print(f"IV={g['iv']:.4f} delta={g['delta']:.4f} gamma={g['gamma']:.6f}")
```
```typescript [TypeScript]
import { allGreeks } from 'thetadatadx';

const g = allGreeks(450.0, 455.0, 0.05, 0.015, 30 / 365, 8.50, 'C');
console.log(`IV=${g.iv.toFixed(4)} delta=${g.delta.toFixed(4)} gamma=${g.gamma.toFixed(6)}`);
```
```go [Go]
g, err := thetadatadx.AllGreeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C")
if err != nil { log.Fatal(err) }
fmt.Printf("IV=%.4f delta=%.4f gamma=%.6f\n", g.IV, g.Delta, g.Gamma)
```
```cpp [C++]
auto g = tdx::all_greeks(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
std::cout << "IV=" << g.iv << " delta=" << g.delta << " gamma=" << g.gamma << std::endl;
```
:::

Result keys across the 23 Greeks: `value`, `delta`, `gamma`, `theta`, `vega`, `rho`, `iv`, `iv_error`, `vanna`, `charm`, `vomma`, `veta`, `speed`, `zomma`, `color`, `ultima`, `d1`, `d2`, `dual_delta`, `dual_gamma`, `epsilon`, `lambda`.

## IV only

When the downstream pricer already has the Greeks it needs but wants IV extracted from a market print.

::: code-group
```rust [Rust]
use thetadatadx::implied_volatility;

let (iv, err) = implied_volatility(450.0, 455.0, 0.05, 0.015, 30.0/365.0, 8.50, "C");
println!("IV={:.4} relative_error={:.6}", iv, err);
```
```python [Python]
from thetadatadx import implied_volatility
iv, err = implied_volatility(450.0, 455.0, 0.05, 0.015, 30/365, 8.50, "C")
```
:::

Solver: bisection, up to 128 iterations. `iv_error` is the relative difference `(theoretical - market) / market` — a sanity check that the solve converged.

## Server-computed Greeks

Where you need historical Greeks over a date range, ThetaData's servers pre-compute them; ThetaDataDx exposes them through the `option_*_greeks_*` endpoint family. The Rust decode core and typed-struct surface keep dense Greeks pulls (176,732 rows × 31 cols on `option_history_greeks_all`) off the GIL and out of Python-object churn.

Pair the server endpoints with the local calculator: pull a chain with server Greeks, then run the local solver for what-if scenarios that would otherwise require a second network round-trip.

## Next

- [Options & Greeks](../options) — full reference: 23 Greeks formulas, chain workflow, wildcard queries
- [DataFrames](./dataframes) — chain `.to_polars()` on an option chain for scenario sweeps in columnar form
