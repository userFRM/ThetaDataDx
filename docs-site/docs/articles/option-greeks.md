---
title: Option Greeks
description: The model behind the Greeks endpoints, with input conventions.
---

# Option Greeks

## Server-computed Greeks

The `option/*/greeks/*` [reference endpoints](/reference/option/snapshot/greeks/all) return Greeks computed by ThetaData using the Black-Scholes model — specifically the formulas catalogued on [Wikipedia's Greeks page](https://en.wikipedia.org/wiki/Greeks_(finance)). Input conventions:

- **Risk-free rate** defaults to SOFR; override the source with `rate_type` (SOFR or a Treasury tenor) or pin an exact rate with `rate_value`.
- **Dividends** are ignored unless you pass `annual_dividend`, a continuous yield as a decimal, or `annual_dividend_amount`, a dividend in dollars per share per year; the server refuses `annual_dividend_amount` beside a non-zero `annual_dividend`.
- **Option price input** is the NBBO midpoint by default; `use_market_value` switches snapshot calculations to the market value, `option_use_nbbo=false` switches the end-of-day endpoints to the last trade price, and trade-Greeks endpoints use the trade price.
- **Underlying price input** is the last underlying trade; `underlyer_use_nbbo` switches to the underlying NBBO midpoint.
- **Implied volatility** is solved numerically from the option price; each row carries an `iv_error` residual so you can judge the fit.
- `version` selects the calculation revision; `latest` uses real time-to-expiration down to a one-hour floor.

Greek-by-Greek field definitions appear on every Greeks reference page's response table.
