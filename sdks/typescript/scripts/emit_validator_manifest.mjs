// Emit `artifacts/validator_typescript.json` from the TypeScript SDK's
// public `index.d.ts` so the cross-language response-shape agreement
// validator (`scripts/validate_agreement.py`) can compare TS field
// names against the runtime first_row dicts the Python / CLI / C++
// SDKs already emit.
//
// Why a manifest instead of a live runtime validator
// ---------------------------------------------------
// Standing up a Node-side runtime validator that calls the napi-rs
// SDK against production with a real account is a multi-day exercise:
// it duplicates the per-endpoint cell matrix (`endpoint_surface.toml`),
// it requires the account credentials at CI time, and it would need
// per-method fixture wiring on top of the JS event loop. The agreement
// validator's job is to catch *response-shape drift*, and shape drift
// shows up at the **return type** boundary -- which the TS SDK
// declares unambiguously in its public `index.d.ts`. Parsing those
// declarations gives us the exact field set every method returns,
// without spending a second on the per-cell live-traffic infrastructure
// the runtime validators carry.
//
// What this script does
// ---------------------
// 1. Parse `index.d.ts` line-by-line.
// 2. Build a map `interfaceName -> Set<fieldName>` for every
//    `export interface X { ... }` block (the tick types and FPSS
//    event payloads).
// 3. Walk every `methodName(...): Array<X> | X` declaration on
//    `Client` and record `{ method, returnType, fields }`.
// 4. Project a representative sub-set of methods (chosen to mirror the
//    "shape drift is most painful" methods called out by the cross-language agreement
//    spec) into the artifact's `records` shape so the existing
//    Python-side diff engine in `scripts/validate_agreement.py`
//    consumes them as-is.
//
// The artifact is emitted at the same path the Python / CLI / C++
// validators use (`artifacts/validator_typescript.json`), keyed by
// `lang: "typescript"`. The Python validator's `LANGS` tuple now
// includes `"typescript"` so the file is loaded automatically.

import fs from 'node:fs';
import path from 'node:path';
import url from 'node:url';

const HERE = path.dirname(url.fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, '..', '..', '..');
const DTS_PATH = path.join(REPO_ROOT, 'sdks', 'typescript', 'index.d.ts');
const ARTIFACT_PATH = path.join(REPO_ROOT, 'artifacts', 'validator_typescript.json');

// Per-method projection: which `Client` methods feed into which
// `(endpoint, mode)` agreement cell. Mirrors the cells that the
// Python / CLI / C++ validators populate so the Python diff engine
// can pair them up. Restricted to the high-signal endpoints
// enumerated in the spec.
const METHOD_TO_CELL = {
  stockSnapshotOHLC: { endpoint: 'stock_snapshot_ohlc', mode: 'concrete' },
  stockSnapshotQuote: { endpoint: 'stock_snapshot_quote', mode: 'concrete' },
  stockSnapshotTrade: { endpoint: 'stock_snapshot_trade', mode: 'concrete' },
  stockHistoryEOD: { endpoint: 'stock_history_eod', mode: 'concrete' },
  stockHistoryOHLC: { endpoint: 'stock_history_ohlc', mode: 'concrete' },
  optionSnapshotQuote: { endpoint: 'option_snapshot_quote', mode: 'concrete' },
  optionSnapshotTrade: { endpoint: 'option_snapshot_trade', mode: 'concrete' },
  optionHistoryOHLC: { endpoint: 'option_history_ohlc', mode: 'concrete' },
};

// Stock and option contract-id fields that the runtime validators
// inject into every first_row. The TS return type is just the tick
// payload, so we annotate the manifest with these so the agreement
// validator's sentinel-stripping logic (omit-vs-null-vs-zero) folds
// the TS shape into the same canonical form as the runtime artifacts.
const STOCK_CONTEXT_FIELDS = ['symbol'];
const OPTION_CONTEXT_FIELDS = ['symbol', 'expiration', 'right', 'strike'];

// Camel -> snake (`bidSize` -> `bid_size`). The runtime artifacts emit
// snake_case keys (Python tick_columnar / CLI raw helpers / C++
// Arrow); TS uses camelCase per napi-rs convention. We normalise here
// so the agreement diff engine compares apples to apples.
function camelToSnake(name) {
  return name.replace(/([A-Z])/g, (_, c) => `_${c.toLowerCase()}`).replace(/^_/, '');
}

function isOptionEndpoint(endpoint) {
  return endpoint.startsWith('option_');
}

function parseInterfaces(source) {
  // `export interface X { ... }` blocks. Returns `Map<name, Set<field>>`.
  // Naive but sufficient: we operate on a generated NAPI declaration
  // file with stable formatting (one field per line, no nested types
  // in tick interfaces).
  const interfaces = new Map();
  const lines = source.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^export interface (\w+)\s*\{?$/);
    if (!m) continue;
    const name = m[1];
    const fields = new Set();
    let j = i + 1;
    let depth = 1;
    while (j < lines.length && depth > 0) {
      const line = lines[j].trim();
      if (line === '}') {
        depth -= 1;
        j += 1;
        continue;
      }
      if (line === '' || line.startsWith('//') || line.startsWith('/*') || line.startsWith('*')) {
        j += 1;
        continue;
      }
      // Field shape: `name?: Type` or `name: Type` -- strip optional
      // marker and trailing punctuation.
      const fm = line.match(/^(\w+)\??:\s*/);
      if (fm) {
        fields.add(fm[1]);
      }
      j += 1;
    }
    interfaces.set(name, fields);
    i = j - 1;
  }
  return interfaces;
}

function parseMethods(source) {
  // Find `methodName(...): ReturnType` declarations on `Client`.
  // The class block is large; we scope the parse to its braces so
  // helper class methods on `FlatFilesNamespace` etc. don't leak in.
  const lines = source.split('\n');
  const methods = new Map();
  let inClass = false;
  let depth = 0;
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trim();
    if (trimmed.startsWith('export declare class Client')) {
      inClass = true;
      depth = 1;
      continue;
    }
    if (!inClass) continue;
    if (trimmed === '}') {
      depth -= 1;
      if (depth === 0) {
        inClass = false;
      }
      continue;
    }
    if (trimmed.endsWith('{')) {
      depth += 1;
    }
    // `methodName(args): RetType`
    const m = trimmed.match(/^(\w+)\s*\([^)]*\)\s*:\s*(.+)$/);
    if (!m) continue;
    const name = m[1];
    if (name === 'static' || name === 'constructor') continue;
    const returnType = m[2].replace(/[;,]\s*$/, '').trim();
    methods.set(name, returnType);
  }
  return methods;
}

function extractTickType(returnType) {
  // `Array<OhlcTick>` -> `OhlcTick`. Returns null if no Tick wrapper.
  const m = returnType.match(/Array<(\w+)>/);
  if (m) return m[1];
  // Plain return types like `OhlcTick` (singular) also handled.
  const single = returnType.match(/^(\w+Tick)$/);
  if (single) return single[1];
  return null;
}

function buildRecords(interfaces, methods) {
  const records = [];
  const skipped = [];
  for (const [methodName, cell] of Object.entries(METHOD_TO_CELL)) {
    const returnType = methods.get(methodName);
    if (!returnType) {
      skipped.push({ method: methodName, reason: 'method not found in index.d.ts' });
      continue;
    }
    const tickType = extractTickType(returnType);
    if (!tickType) {
      skipped.push({ method: methodName, reason: `non-tick return type: ${returnType}` });
      continue;
    }
    const tickFields = interfaces.get(tickType);
    if (!tickFields) {
      skipped.push({ method: methodName, reason: `tick interface ${tickType} not found` });
      continue;
    }
    const contextFields = isOptionEndpoint(cell.endpoint) ? OPTION_CONTEXT_FIELDS : STOCK_CONTEXT_FIELDS;
    // Synthetic first_row carrying the snake_case field SET. The
    // values are sentinel placeholders the agreement validator
    // canonicalises away (date 0 -> None, ms_of_day -1 -> None,
    // strike 0.0 -> None, right "" -> None) so the comparison
    // collapses to "do the runtime SDKs emit the same key set the
    // TS public surface advertises?". This is the load-bearing
    // assertion: TS shape drift would surface as a missing or new
    // field name relative to the runtime artifacts.
    const firstRow = {};
    for (const f of tickFields) {
      const snake = camelToSnake(f);
      // Pick a sentinel that the validator strips for sentinel-shaped
      // fields and a benign placeholder otherwise. The agreement
      // validator's `_canonicalize_row` collapses these to None for
      // contract-id fields and round-trips them as-is otherwise.
      if (snake === 'date' || snake.endsWith('_date') || snake === 'expiration') {
        firstRow[snake] = 0;
      } else if (
        snake === 'ms_of_day' ||
        snake.endsWith('_ms_of_day') ||
        snake.endsWith('_time') ||
        snake === 'time'
      ) {
        firstRow[snake] = -1;
      } else if (snake === 'strike' || snake.endsWith('_strike')) {
        firstRow[snake] = 0.0;
      } else if (snake === 'right' || snake.endsWith('_right')) {
        firstRow[snake] = '';
      } else {
        firstRow[snake] = null;
      }
    }
    for (const f of contextFields) {
      if (!(f in firstRow)) {
        firstRow[f] = f === 'strike' ? 0.0 : f === 'right' ? '' : null;
      }
    }
    records.push({
      endpoint: cell.endpoint,
      mode: cell.mode,
      rationale: `TypeScript public-surface shape manifest for ${methodName} -> ${tickType}`,
      status: 'PASS',
      row_count: 1,
      duration_ms: 0,
      detail: 'shape manifest extracted from sdks/typescript/index.d.ts',
      first_row: firstRow,
    });
  }
  return { records, skipped };
}

function main() {
  if (!fs.existsSync(DTS_PATH)) {
    console.error(`error: ${DTS_PATH} not found`);
    process.exit(1);
  }
  const source = fs.readFileSync(DTS_PATH, 'utf8');
  const interfaces = parseInterfaces(source);
  const methods = parseMethods(source);
  const { records, skipped } = buildRecords(interfaces, methods);

  if (skipped.length > 0) {
    console.error('warning: some method projections were skipped:');
    for (const s of skipped) {
      console.error(`  ${s.method}: ${s.reason}`);
    }
  }

  if (records.length === 0) {
    console.error('error: no records emitted; did the index.d.ts shape change?');
    process.exit(1);
  }

  fs.mkdirSync(path.dirname(ARTIFACT_PATH), { recursive: true });
  fs.writeFileSync(
    ARTIFACT_PATH,
    JSON.stringify(
      { lang: 'typescript', records },
      null,
      2,
    ),
  );
  console.log(`emitted ${records.length} records to ${ARTIFACT_PATH}`);
}

main();
