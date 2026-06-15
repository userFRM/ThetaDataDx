# thetadatadx (TypeScript / Node.js)

The Node.js SDK for [ThetaData](https://thetadata.us) market data. Pull US stock, option, index, and rate data three ways — point-in-time **history**, real-time **streaming**, and whole-universe **flat files** — all from a single authenticated client. Connects straight to ThetaData; no Java terminal, no JVM, no local proxy.

[![npm](https://img.shields.io/npm/v/thetadatadx?logo=npm)](https://www.npmjs.com/package/thetadatadx)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/userFRM/ThetaDataDx/blob/main/LICENSE)
[![Node](https://img.shields.io/badge/node-20%2B-339933.svg?logo=node.js&logoColor=white)](https://nodejs.org)
[![Discord](https://img.shields.io/badge/Discord-community-5865F2.svg?logo=discord&logoColor=white)](https://discord.thetadata.us/)

> [!IMPORTANT]
> A valid [ThetaData](https://thetadata.us) subscription is required. The SDK
> authenticates against ThetaData's Nexus API using your account credentials.

## Features

- **Complete coverage** — stocks, options, indices, and rates across 61 typed endpoints.
- **Three access modes, one client** — point-in-time history, real-time streaming, and bulk flat-file downloads.
- **Fully typed** — every endpoint, tick, and streaming event ships with hand-checked `.d.ts` declarations.
- **Greeks on demand** — five tiers of Black-Scholes Greeks and implied volatility, served straight from the option endpoints.
- **Arrow on the way out** — flat-file results emit Arrow IPC for a zero-copy handoff to `apache-arrow`.
- **No terminal to run** — prebuilt native binaries; nothing to compile, nothing to babysit locally.

## Install

```bash
npm install thetadatadx
```

Prebuilt binaries are downloaded automatically for Linux x64 (glibc), macOS arm64 (Apple Silicon), and Windows x64 (MSVC). No Rust toolchain is required.

## Quick start

> [!TIP]
> Credentials can come from a `creds.txt` file (email on line 1, password on
> line 2) via `connectFromFile`, or inline via `connect(email, password)`.

```typescript
import { Client } from 'thetadatadx';

const tdx = Client.connectFromFile('creds.txt');

// First-order Greeks for every strike on SPY's 2026-06-19 expiry, as of 2024-03-15
const greeks = await tdx.optionHistoryGreeksFirstOrder('SPY', '20260619', '20240315');
for (const t of greeks.slice(0, 5)) {
  console.log(`K=${t.strike} ${t.right} delta=${t.delta.toFixed(4)} theta=${t.theta.toFixed(4)}`);
}
```

Every historical method resolves a `Promise` of typed tick objects off the runtime's execution thread, so a fetch never holds the event loop:

```typescript
const eod = await tdx.stockHistoryEOD('AAPL', '20240101', '20240301');
console.log(eod.length, eod[0].close);

const bars = await tdx.stockHistoryOHLC('AAPL', '20240315', { interval: '60000' });
const exps = await tdx.optionListExpirations('SPY');

// Optional parameters — including a per-call timeout — ride in the trailing options object
const snap = await tdx.stockSnapshotQuote(['AAPL', 'MSFT'], { timeoutMs: 5000 });
```

## Streaming

Real-time quotes and trades flow through the same client. Register a callback with `startStreaming`; events are discriminated on `event.kind` and the typed payload narrows automatically:

```typescript
import { Contract, Client } from 'thetadatadx';

const tdx = Client.connectFromFile('creds.txt');
const formatContract = (contract: {
  symbol: string;
  expiration?: number;
  strike?: number;
  right?: string;
}) => [contract.symbol, contract.expiration, contract.strike, contract.right]
  .filter((value) => value != null)
  .join(' ');

tdx.startStreaming((event) => {
  if (event.kind === 'trade' && event.trade) {
    const { contract, price, size, exchange, msOfDay, sequence, condition } = event.trade;
    console.log(
      `${formatContract(contract)} trade price=${price} size=${size} ` +
      `exchange=${exchange} ms_of_day=${msOfDay} sequence=${sequence} condition=${condition}`,
    );
  } else if (event.kind === 'quote' && event.quote) {
    const { contract, bid, ask, bidSize, askSize, bidExchange, askExchange, msOfDay } = event.quote;
    console.log(
      `${formatContract(contract)} quote bid=${bid} ask=${ask} ` +
      `bid_size=${bidSize} ask_size=${askSize} bid_exchange=${bidExchange} ` +
      `ask_exchange=${askExchange} ms_of_day=${msOfDay}`,
    );
  }
});

const leg = { expiration: '20260620', strike: '550', right: 'C' };
tdx.subscribeMany([
  Contract.option('SPY', leg).quote(),
  Contract.option('SPY', leg).trade(),
]);
```

Build subscriptions with the fluent `Contract` API and pass them — one at a time or in bulk — to `subscribe` / `subscribeMany`. Every subscription is the same typed value, so quotes, trades, and open interest across contracts mix freely in one array:

```typescript
import { Contract, SecType } from 'thetadatadx';

const stock = Contract.stock('AAPL');
const option = Contract.option('SPY', { expiration: '20260620', strike: '550', right: 'C' });

tdx.subscribe(stock.quote());
tdx.subscribeMany([option.quote(), option.trade(), option.openInterest()]);
```

Or take a whole-market feed — every option trade across the universe, no per-contract setup:

```typescript
import { SecType } from 'thetadatadx';

tdx.subscribe(SecType.option().fullTrades());   // the callback runs per event — keep it fast
```

When you are done, stop the stream and drain it. By the time `awaitDrain` resolves, the callback has stopped firing, so any state it closed over can be released safely:

```typescript
tdx.stopStreaming();
const drained = await tdx.awaitDrain(5000);
```

> [!TIP]
> On an involuntary disconnect the client recovers on its own — exponential
> backoff with jitter, host failover, then a paced re-subscribe of every active
> contract.

## Types

Every tick type and streaming event is exported. Import the ones you need:

```typescript
import type { OhlcTick, GreeksAllTick, Quote, Trade, StreamEvent } from 'thetadatadx';
```

The streaming callback receives a discriminated `StreamEvent`, narrowed on `event.kind`. Market-data events (`trade`, `quote`, `ohlcvc`, `open_interest`) carry their payload under a matching field; one typed payload also exists per lifecycle event (`connected`, `loginSuccess`, `disconnected`, `reconnecting`, …):

```typescript
tdx.startStreaming((event: StreamEvent) => {
  switch (event.kind) {
    case 'trade':         /* event.trade is Trade */                break;
    case 'quote':         /* event.quote is Quote */                break;
    case 'ohlcvc':        /* event.ohlcvc is Ohlcvc */              break;
    case 'open_interest': /* event.openInterest is OpenInterest */  break;
  }
});
```

`kind` is a string-literal union (not a `const enum`), so the type information stays self-contained under `"isolatedModules": true` and works across Vite, esbuild, ts-jest, and Next.js.

> [!NOTE]
> Wherever a 64-bit integer crosses the boundary it surfaces as `bigint`, not
> `number` — `volume` and `count` on OHLC / EOD ticks, `droppedEventCount()`,
> and `receivedAtNs` on every event. Use `42n` literals for comparisons, or
> widen with `Number(x)` at the point of display.

## Flat files

Whole-universe daily snapshots for one `(security type, request type, date)` at a time. The decoded schema follows the request type, so the binding emits Arrow IPC bytes — pair with `apache-arrow`'s `tableFromIPC` to materialise a typed `Table`:

```typescript
import { Client } from 'thetadatadx';
import { tableFromIPC } from 'apache-arrow';   // peer dependency

const tdx = Client.connectFromFile('creds.txt');

const rows = tdx.flatFiles.optionQuote('20260428');
console.log(rows.len());

const table = tableFromIPC(rows.toArrowIpc());
// Or skip Arrow entirely: const json = JSON.parse(rows.toJson());

// Generic dispatcher when security type / request type come from config
const oi = tdx.flatFiles.request('OPTION', 'OPEN_INTEREST', '20260428');

// Or write the raw vendor file straight to disk
tdx.flatFileToPath('OPTION', 'QUOTE', '20260428', '/tmp/option-quote', 'csv');
```

Available `flatFiles.*` methods: `optionQuote`, `optionTrade`, `optionTradeQuote`, `optionOhlc`, `optionOpenInterest`, `optionEod`, `stockQuote`, `stockTrade`, `stockTradeQuote`, `stockEod`, plus `request(secType, reqType, date)`.

## Endpoint coverage

61 typed endpoints across stocks, options, indices, the market calendar, and interest rates, plus real-time streaming.

| Category | Endpoints | Examples |
|---|---|---|
| Stock | 14 | EOD, OHLC, trades, quotes, snapshots, at-time |
| Option | 34 | Every stock surface plus five Greeks tiers, open interest, contract lists |
| Index | 9 | EOD, OHLC, price, snapshots |
| Calendar | 3 | Market open/close, holidays, early closes |
| Interest rate | 1 | EOD rate history |

Every endpoint is a camelCase method on `Client`. The full method list with JSDoc lives in `index.d.ts` and the [API reference](https://userfrm.github.io/ThetaDataDx/reference/).

## Errors

Every call rejects with a typed error under a common `ThetaDataError` base — authentication, rate limit, not found, deadline exceeded, invalid parameter, and the rest — so the same cases are catchable here exactly as they are in every other binding.

## Building from source

Only needed when a platform has no prebuilt binary or you are developing locally:

```bash
cd sdks/typescript
npm install
npm run build          # requires Rust stable + protoc
```

## Documentation

- [Documentation site](https://userfrm.github.io/ThetaDataDx/) — getting started, API reference, streaming, flat files
- [Repository, issues, contributing](https://github.com/userFRM/ThetaDataDx)
- Community discussion on the [ThetaData Discord](https://discord.thetadata.us/)

## License

Licensed under the Apache License, Version 2.0.
