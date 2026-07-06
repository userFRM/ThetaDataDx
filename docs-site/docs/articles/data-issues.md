---
title: Data Issues?
description: A short checklist before filing a data problem.
---

# Data Issues?

Work down this list before reporting a problem — most "missing data" reports resolve at one of these steps.

::: details Empty response on a snapshot endpoint
Snapshot caches reset at midnight ET and fill as messages arrive. On weekends and holidays there may be nothing to serve — use the matching `history` endpoint with the last trading day instead. `calendar/open_today` tells you whether the session is live.
:::

::: details Empty response on an option request
Check the contract actually existed and traded: list [expirations](/reference/option/list/expirations) and [strikes](/reference/option/list/strikes) for the underlying, then confirm with [dates](/reference/option/list/dates) that data exists for your request type on your date. A `NoDataFoundError` for a quiet contract on a quiet day is a normal outcome.
:::

::: details Wrong or surprising symbol
Index option roots split by settlement (`SPX` vs `SPXW`) — see [Symbology](/articles/symbology). For stocks, confirm the ticker existed on your dates (ticker changes, delistings, splits).
:::

::: details Permission errors
A `SubscriptionError` means the endpoint requires a higher tier on that asset class — check the badge on the endpoint's reference page against [Subscriptions](/articles/subscriptions).
:::

::: details Quotes and trades look misaligned
Trade and quote feeds are separate streams. For trade-with-prevailing-quote analysis, use the `trade_quote` endpoints, which pair each trade with the NBBO at or before the trade timestamp (`exclusive` controls the boundary).
:::

::: details A value disagrees with another vendor
Check the `condition` / `exchange` codes on the rows in question against the [Trade](/articles/trade-conditions) / [Quote](/articles/quote-conditions) conditions and [Exchanges](/articles/exchanges) tables — many discrepancies are condition-filtering differences (odd lots, late reports, derivative prints).
:::

Still stuck? Report the exact request (endpoint, parameters, date) on the [issue tracker](https://github.com/userFRM/ThetaDataDx/issues). For upstream data-content questions, use ThetaData's support channels listed on [their documentation site](https://docs.thetadata.us/).
