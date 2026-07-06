// Fluent contract-first streaming example.
//
// Demonstrates the primary streaming surface on the TS SDK: typed
// `Contract` / `Subscription` values feeding the polymorphic
// `client.stream.subscribe(...)` and `client.stream.subscribeMany(...)` paths,
// plus the push-callback delivery contract.
//
// Run with valid `creds.txt` in the working directory:
//
//     npx tsx fluent_streaming_quote.ts

import { Contract, SecType, Client } from "thetadatadx-ts";

async function main(): Promise<void> {
  const client = await Client.connectFromFile("creds.txt");

  // Fluent contract-first construction.
  const stock = Contract.stock("AAPL");
  const option = Contract.option("SPY", { expiration: "20260620", strike: "550", right: "C" });

  // Register the per-event callback. The napi-rs binding hands every
  // streaming event to the JS callback on the Node main thread via a
  // `ThreadsafeFunction`, so the libuv loop stays responsive.
  await client.stream.startStreaming((event) => {
    switch (event.kind) {
      case "trade": {
        const trade = event.trade!;
        console.log(
          `[${trade.contract.symbol}] TRADE ${trade.price.toFixed(2)} x ${trade.size}`,
        );
        break;
      }
      case "quote": {
        const quote = event.quote!;
        console.log(
          `[${quote.contract.symbol}] QUOTE bid=${quote.bid.toFixed(2)} ask=${quote.ask.toFixed(2)}`,
        );
        break;
      }
      case "ohlcvc": {
        // The full-trade stream sends a quote and an OHLC bar before each
        // trade, so the same callback also receives ohlcvc bars.
        const bar = event.ohlcvc!;
        console.log(
          `[${bar.contract.symbol}] BAR o=${bar.open.toFixed(2)} h=${bar.high.toFixed(2)} l=${bar.low.toFixed(2)} c=${bar.close.toFixed(2)}`,
        );
        break;
      }
      default:
        break;
    }
  });

  try {
    // One subscription at a time.
    client.stream.subscribe(stock.quote());
    client.stream.subscribe(stock.trade());

    // Or many at once.
    client.stream.subscribeMany([
      option.quote(),
      option.trade(),
      option.openInterest(),
    ]);

    // Full-stream — every option trade across the universe.
    client.stream.subscribe(SecType.option().fullTrades());

    // Let events flow for 60 s.
    await new Promise((r) => setTimeout(r, 60_000));
  } finally {
    client.stream.stopStreaming();
    await client.stream.awaitDrain(5000);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
