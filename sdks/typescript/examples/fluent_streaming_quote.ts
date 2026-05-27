// Fluent contract-first streaming example — ported from
// `sdks/python/examples/fluent_streaming_quote.py` for U10.
//
// Demonstrates the primary documented streaming surface on the TS
// SDK: typed `Contract` / `Subscription` values feeding the
// polymorphic `client.subscribe(...)` and `client.subscribeMany(...)`
// paths, plus the async-iterator delivery mode that mirrors the
// Python `with client.streaming(callback) as session` block.
//
// Run with valid `creds.txt` in the working directory:
//
//     npx tsx fluent_streaming_quote.ts
//
// The TypeScript SDK exposes FPSS events via `[Symbol.asyncIterator]`
// on the iterator-mode session — the napi-rs binding does not ship a
// push-callback variant for JS (the `setCallback` semantics in
// Node.js would block the libuv loop on every event).

import {
  Credentials,
  Config,
  Contract,
  SecType,
  ThetaDataDxClient,
} from "thetadatadx";

async function main(): Promise<void> {
  const creds = Credentials.fromFile("creds.txt");
  const config = Config.production();
  const client = new ThetaDataDxClient(creds, config);

  // Fluent contract-first construction.
  const stock = Contract.stock("AAPL");
  const option = Contract.option("SPY", "20260620", "550", "C");

  // Open the streaming connection.
  client.startStreamingIter();
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

    // Drain a few events from the async iterator.
    const iter = client.eventIterator();
    const deadline = Date.now() + 60_000; // 60 s
    while (Date.now() < deadline) {
      const event = iter.tryNext();
      if (event === null) {
        await new Promise((r) => setTimeout(r, 10));
        continue;
      }
      switch (event.kind) {
        case "trade":
          console.log(
            `[${event.contract.symbol}] TRADE ${event.price.toFixed(2)} x ${event.size}`,
          );
          break;
        case "quote":
          console.log(
            `[${event.contract.symbol}] QUOTE bid=${event.bid.toFixed(2)} ask=${event.ask.toFixed(2)}`,
          );
          break;
        default:
          break;
      }
    }
  } finally {
    client.stopStreaming();
    client.awaitDrain(5000);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
