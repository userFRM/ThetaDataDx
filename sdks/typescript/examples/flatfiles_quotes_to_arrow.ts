// FLATFILES dynamic-schema decode -> Arrow IPC -> apache-arrow Table.
//
// The flat-file surface returns whole-universe data for a single
// (SecType, ReqType, date) tuple. Decoded shape is determined at
// runtime by the request type, so the binding emits Arrow IPC bytes
// and the caller deserialises with `apache-arrow`'s `tableFromIPC`.
//
// Run with: `npx tsx flatfiles_quotes_to_arrow.ts`
// Requires: `npm i apache-arrow` (peer dep, not bundled by thetadatadx).

import { ThetaDataDxClient } from "thetadatadx";
import { tableFromIPC } from "apache-arrow";

const tdx = ThetaDataDxClient.connectFromFile("creds.txt");

// Whole-universe option quotes for one trading day.
const rows = tdx.flatFiles.optionQuote("20260428");
console.log(`option_quote rows: ${rows.len()}`);

// Apache Arrow table -- one column per vendor field plus the contract
// key columns (symbol, expiration, strike, right). Schema inferred
// from the first row by `flatfiles::arrow::rows_to_arrow`.
const ipc = rows.toArrowIpc();
const table = tableFromIPC(ipc);
console.log(table.schema.fields.map((f) => `${f.name}:${f.type}`).join(", "));

// Same path, dispatched dynamically.
const oi = tdx.flatFiles.request("OPTION", "OPEN_INTEREST", "20260428");
console.log(`open_interest rows: ${oi.len()}`);

// Drop raw vendor CSV bytes to disk without materialising rows.
const path = tdx.flatFileToPath(
  "OPTION",
  "QUOTE",
  "20260428",
  "/tmp/option-quote",
  "csv",
);
console.log(`raw vendor CSV at ${path}`);
