// Standalone `MddsClient` structural contract (offline — no connect).
//
// `MddsClient` is the historical-only napi handle: the same MDDS/Nexus
// surface as the unified `ThetaDataDxClient`, with no streaming methods
// reachable. It mirrors the Python `MddsClient`
// (`sdks/python/src/mdds_client.rs`), the C++ `tdx::Client`, and the C
// ABI `tdx_client_*` entry points. These assertions pin the split
// structurally against `index.d.ts` and the loaded addon so a generator
// or lib.rs change that leaks an FPSS method onto the MDDS surface — or
// drops the historical surface from it — fails here without live
// credentials.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const dts = readFileSync(resolve(__dirname, '..', 'index.d.ts'), 'utf8');

let mod;
try {
  mod = await import('../index.js');
} catch {
  console.error('FAIL: native addon not built; run `npm run build` first');
  process.exit(1);
}

// The `MddsClient` class body as declared in `index.d.ts`.
const mddsBlock = dts.match(/export declare class MddsClient \{[\s\S]*?\n\}/);
const unifiedBlock = dts.match(
  /export declare class ThetaDataDxClient \{[\s\S]*?\n\}/
);

function methodNames(block) {
  // Collect declared `methodName(` / `static methodName(` lines.
  return new Set(
    block
      .split('\n')
      .map((line) => line.match(/^\s+(?:static\s+)?([a-z][a-zA-Z0-9]*)\(/))
      .filter(Boolean)
      .map((m) => m[1])
  );
}

describe('MddsClient native addon surface', () => {
  it('exports the MddsClient class with the creds-first connect factories', () => {
    assert.ok(mod.MddsClient, 'MddsClient should be exported');
    for (const factory of ['connect', 'connectFromFile']) {
      assert.equal(
        typeof mod.MddsClient[factory],
        'function',
        `MddsClient.${factory} should be a static factory`
      );
    }
  });

  it('declares the MddsClient class in index.d.ts', () => {
    assert.ok(mddsBlock, 'MddsClient class missing from index.d.ts');
    assert.ok(unifiedBlock, 'ThetaDataDxClient class missing from index.d.ts');
  });
});

describe('MddsClient carries the historical surface', () => {
  it('exposes the buffered data-fetch families', () => {
    const methods = methodNames(mddsBlock[0]);
    for (const expected of [
      'stockHistoryEOD',
      'optionHistoryGreeksAll',
      'stockSnapshotQuote',
      'optionListContracts',
      'calendarOnDate',
      'interestRateHistoryEOD',
    ]) {
      assert.ok(
        methods.has(expected),
        `MddsClient must expose historical method ${expected}`
      );
    }
  });

  it('exposes the server-stream companions', () => {
    const methods = methodNames(mddsBlock[0]);
    assert.ok(
      methods.has('stockHistoryEODStream'),
      'MddsClient must expose the stream companion stockHistoryEODStream'
    );
  });

  it('matches the unified client on the historical surface (lockstep)', () => {
    const mdds = methodNames(mddsBlock[0]);
    const unified = methodNames(unifiedBlock[0]);
    // Every method the MDDS-only client exposes must also exist on the
    // unified client — the historical surface is generated identically
    // onto both, so the MDDS set is a strict subset of the unified set.
    for (const name of mdds) {
      assert.ok(
        unified.has(name),
        `MddsClient method ${name} is missing from ThetaDataDxClient — the two historical surfaces have drifted`
      );
    }
  });
});

describe('MddsClient never exposes the FPSS / streaming surface', () => {
  // The streaming, subscription, lifecycle, and ring-telemetry methods
  // live only on the unified client. An MDDS-only handle that surfaced
  // any of these could open or observe an FPSS slot, defeating the split.
  // This is the TypeScript parity of the Python `MddsClient` block-list
  // (`FPSS_TOUCHING_METHODS` in `sdks/python/src/mdds_client.rs`).
  const FPSS_TOUCHING = [
    'startStreaming',
    'stopStreaming',
    'shutdown',
    'reconnect',
    'isStreaming',
    'awaitDrain',
    'subscribe',
    'subscribeMany',
    'unsubscribe',
    'unsubscribeMany',
    'activeSubscriptions',
    'activeFullSubscriptions',
    'droppedEventCount',
    'panicCount',
    'ringOccupancy',
    'ringCapacity',
    'millisSinceLastEvent',
    'lastEventReceivedAtUnixNanos',
    'lastConnectedAddr',
  ];

  it('declares none of the FPSS-touching methods', () => {
    const methods = methodNames(mddsBlock[0]);
    for (const banned of FPSS_TOUCHING) {
      assert.ok(
        !methods.has(banned),
        `MddsClient must NOT expose the FPSS-touching method ${banned}`
      );
    }
  });

  it('the unified client retains those methods (control)', () => {
    // Guard the guard: confirm the banned names are real methods on the
    // unified client, so the absence check above is meaningful and not
    // passing on a renamed/removed method.
    const unified = methodNames(unifiedBlock[0]);
    for (const present of ['startStreaming', 'subscribe', 'ringOccupancy']) {
      assert.ok(
        unified.has(present),
        `ThetaDataDxClient should still expose ${present}`
      );
    }
  });
});
