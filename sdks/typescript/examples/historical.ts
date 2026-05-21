// Fetch historical stock and option data from ThetaData via the Rust
// SDK — ported from `sdks/python/examples/historical.py` for U10.
//
// Run with valid `creds.txt` (line 1 = email, line 2 = password) in
// the working directory:
//
//     npx tsx historical.ts
//
// Requires the prebuilt napi binding (`npm install thetadatadx`)
// plus `npm install --save-dev tsx` for the runner.

import { Credentials, Config, ThetaDataDxClient } from "thetadatadx";

const creds = Credentials.fromFile("creds.txt");
const client = new ThetaDataDxClient(creds, Config.production());

// End-of-day stock data.
console.log("=== AAPL EOD (Jan-Mar 2024) ===");
const eod = client.stockHistoryEOD("AAPL", "20240101", "20240301");
for (const tick of eod.slice(0, 5)) {
  console.log(
    `  ${tick.date}: O=${tick.open.toFixed(2)} H=${tick.high.toFixed(2)} ` +
      `L=${tick.low.toFixed(2)} C=${tick.close.toFixed(2)} V=${tick.volume}`,
  );
}
console.log(`  ... ${eod.length} total days\n`);

// Intraday 1-minute bars.
console.log("=== AAPL 1-min OHLC (Mar 15, 2024) ===");
const bars = client.stockHistoryOhlc("AAPL", "20240315", "60000");
for (const bar of bars.slice(0, 5)) {
  console.log(
    `  ${bar.msOfDay}ms: O=${bar.open.toFixed(2)} H=${bar.high.toFixed(2)} ` +
      `L=${bar.low.toFixed(2)} C=${bar.close.toFixed(2)}`,
  );
}
console.log(`  ... ${bars.length} total bars\n`);

// Option expirations.
console.log("=== SPY Option Expirations ===");
const exps = client.optionListExpirations("SPY");
console.log(`  Next 5: ${exps.slice(0, 5).join(", ")}\n`);

// Option strikes.
if (exps.length > 0) {
  const strikes = client.optionListStrikes("SPY", exps[0]);
  console.log(`=== SPY ${exps[0]} Strikes ===`);
  console.log(
    `  ${strikes.length} strikes, range: ${strikes[0]} - ${strikes[strikes.length - 1]}`,
  );
}
