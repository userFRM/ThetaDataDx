---
title: TypeScript / Node.js Quickstart
description: Install, authenticate, run a historical call, subscribe to streaming, and handle errors with ThetaDataDx in TypeScript or Node.js.
---

# TypeScript / Node.js Quickstart

`napi-rs`-based native addon — no separate FFI library to build, no WASM. Works in Node.js 18+.

## Install

```bash
npm install thetadatadx
```

From source (requires Rust toolchain):

```bash
git clone https://github.com/userFRM/ThetaDataDx.git
cd ThetaDataDx/sdks/typescript
npm install
npm run build
```

## Authenticate and connect

```typescript
import { ThetaDataDx } from 'thetadatadx';

// From file
const tdx = await ThetaDataDx.connectFromFile('creds.txt');

// Or from env vars
const tdxEnv = await ThetaDataDx.connect(
    process.env.THETA_EMAIL!,
    process.env.THETA_PASS!,
);
```

## Historical call

```typescript
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

const eod = tdx.stockHistoryEod('AAPL', '20240101', '20240301');
for (const tick of eod) {
    console.log(`${tick.date}: O=${tick.open} H=${tick.high} L=${tick.low} C=${tick.close} V=${tick.volume}`);
}
```

Method names are `lowerCamelCase` versions of the Rust snake_case surface. Arguments match the Rust signature one-to-one.

## Streaming call

```typescript
import { ThetaDataDx } from 'thetadatadx';

const tdx = await ThetaDataDx.connectFromFile('creds.txt');

tdx.startStreaming();
tdx.subscribeQuotes('AAPL');
tdx.subscribeTrades('MSFT');

try {
    while (true) {
        const event = tdx.nextEvent(1000);
        if (!event) continue;

        if (event.kind === 'quote') {
            console.log(`Quote: ${event.contractId} ${event.bid.toFixed(2)}/${event.ask.toFixed(2)}`);
        } else if (event.kind === 'trade') {
            console.log(`Trade: ${event.contractId} ${event.price.toFixed(2)} x ${event.size}`);
        } else if (event.kind === 'simple' && event.eventType === 'disconnected') {
            break;
        }
    }
} finally {
    tdx.stopStreaming();
}
```

## Error handling

```typescript
import {
    ThetaDataError, AuthError, RateLimitError, SubscriptionError,
} from 'thetadatadx';

try {
    const ticks = tdx.optionHistoryGreeksAll('SPY', '20240419', '500', 'C',
                                             '20240101', '20240301');
} catch (e) {
    if (e instanceof RateLimitError) {
        await new Promise(r => setTimeout(r, e.waitMs));
        // retry
    } else if (e instanceof SubscriptionError) {
        console.error(`${e.endpoint} requires ${e.requiredTier}`);
    } else if (e instanceof AuthError) {
        await refreshCredentials();
    } else {
        throw e;
    }
}
```

## Next

- [Historical data](../historical/) — 61 endpoints
- [Streaming (FPSS)](../streaming/) — polling model, event types, reconnect
- [Options & Greeks](../options) — wildcard chain queries, local Greeks calculator
- [Error handling](../getting-started/errors) — full `ThetaDataError` hierarchy
