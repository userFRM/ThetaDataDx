// Fluent contract-first streaming example.
//
// Demonstrates the primary streaming surface on the TS SDK: typed
// `Contract` / `Subscription` values feeding the polymorphic
// `client.subscribe(...)` and `client.subscribeMany(...)` paths,
// plus the push-callback delivery contract.
//
// Run with valid `creds.txt` in the working directory:
//
//     npx tsx fluent_streaming_quote.ts

import { Contract, SecType, Client } from "thetadatadx";

async function main(): Promise<void> {
  const client = Client.connectFromFile("creds.txt");

  // Fluent contract-first construction.
  const stock = Contract.stock("AAPL");
  const option = Contract.option("SPY", { expiration: "20260620", strike: "550", right: "C" });

  // Register the per-event callback. The napi-rs binding hands every
  // FPSS event to the JS callback on the Node main thread via a
  // `ThreadsafeFunction`, so the libuv loop stays responsive.
  client.startStreaming((event) => {
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
      default:
        break;
    }
  });

  try {
    // One subscription at a time.
    client.subscribe(stock.quote());
    client.subscribe(stock.trade());

    // Or many at once.
    client.subscribeMany([
      option.quote(),
      option.trade(),
      option.openInterest(),
    ]);

    // Full-stream — every option trade across the universe.
    client.subscribe(SecType.option().fullTrades());

    // Let events flow for 60 s.
    await new Promise((r) => setTimeout(r, 60_000));
  } finally {
    client.stopStreaming();
    await client.awaitDrain(5000);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
