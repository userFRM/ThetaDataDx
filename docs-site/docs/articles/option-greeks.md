---
title: Option Greeks
description: The model behind the Greeks endpoints and the local calculator, with input conventions.
---

# Option Greeks

## Server-computed Greeks

The `option/*/greeks/*` [reference endpoints](/reference/option/snapshot/greeks/all) return Greeks computed by ThetaData using the Black-Scholes model — specifically the formulas catalogued on [Wikipedia's Greeks page](https://en.wikipedia.org/wiki/Greeks_(finance)). Input conventions:

- **Risk-free rate** defaults to SOFR; override the source with `rate_type` (SOFR or a Treasury tenor) or pin an exact rate with `rate_value`.
- **Dividends** are ignored unless you pass `annual_dividend`.
- **Option price input** is the NBBO midpoint by default; `use_market_value` switches snapshot calculations to the market value, and trade-Greeks endpoints use the trade price.
- **Underlying price input** is the last underlying trade; `underlyer_use_nbbo` switches to the underlying NBBO midpoint.
- **Implied volatility** is solved numerically from the option price; each row carries an `iv_error` residual so you can judge the fit.
- `version` selects the calculation revision; `latest` uses real time-to-expiration down to a one-hour floor.

## Local calculator

The SDK also ships an offline Black-Scholes calculator — `all_greeks(...)` and `implied_volatility(...)` in every language — that computes the value, first- through third-order Greeks (delta, gamma, theta, vega, rho, vanna, charm, vomma, veta, speed, zomma, color, ultima, epsilon, lambda, vera, dual delta, dual gamma, d1, d2), and IV via bisection, with no server round trip and no subscription.

```python
from thetadatadx import all_greeks

g = all_greeks(spot=450.0, strike=455.0, rate=0.05, div_yield=0.015,
               tte=30 / 365, option_price=8.50, right="C")
print(g.iv, g.delta, g.gamma)
```

Inputs are spot, strike, rate, dividend yield (both as decimals: `0.05` = 5%), time to expiration in years, the market option price, and the right. The same seven arguments drive the MCP server's offline `iv` / `greeks` tools.

Greek-by-Greek field definitions appear on every Greeks reference page's response table.
