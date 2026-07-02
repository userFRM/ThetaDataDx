// Standalone `HistoricalClient` structural contract (offline — no connect).
//
// `HistoricalClient` is the historical-only napi handle: the same MDDS/Nexus
// surface as the unified `Client`, with no streaming methods
// reachable. It mirrors the Python `HistoricalClient`
// (`thetadatadx-py/src/mdds_client.rs`), the C++ `thetadatadx::Client`, and the C
// ABI `thetadatadx_client_*` entry points. These assertions pin the split
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

// The `HistoricalClient` class body as declared in `index.d.ts`.
const mddsBlock = dts.match(/export declare class HistoricalClient \{[\s\S]*?\n\}/);
// The unified client's historical surface lives on the `client.historical`
// `HistoricalView` view, and its streaming surface on the `client.stream`
// `StreamView` view.
const historicalViewBlock = dts.match(
  /export declare class HistoricalView \{[\s\S]*?\n\}/
);
const streamViewBlock = dts.match(
  /export declare class StreamView \{[\s\S]*?\n\}/
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

describe('HistoricalClient native addon surface', () => {
  it('exports the HistoricalClient class with the creds-first connect factories', () => {
    assert.ok(mod.HistoricalClient, 'HistoricalClient should be exported');
    for (const factory of ['connect', 'connectFromFile']) {
      assert.equal(
        typeof mod.HistoricalClient[factory],
        'function',
        `HistoricalClient.${factory} should be a static factory`
      );
    }
  });

  it('declares the HistoricalClient class in index.d.ts', () => {
    assert.ok(mddsBlock, 'HistoricalClient class missing from index.d.ts');
    assert.ok(historicalViewBlock, 'HistoricalView class missing from index.d.ts');
    assert.ok(streamViewBlock, 'StreamView class missing from index.d.ts');
  });
});

describe('HistoricalClient carries the historical surface', () => {
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
        `HistoricalClient must expose historical method ${expected}`
      );
    }
  });

  it('exposes the server-stream companions', () => {
    const methods = methodNames(mddsBlock[0]);
    assert.ok(
      methods.has('stockHistoryEODStream'),
      'HistoricalClient must expose the stream companion stockHistoryEODStream'
    );
  });

  it('matches the unified client on the historical surface (lockstep)', () => {
    const mdds = methodNames(mddsBlock[0]);
    const historical = methodNames(historicalViewBlock[0]);
    // The `connect` / `connectFromFile` static factories are lifecycle
    // constructors that live only on the standalone client; the
    // `HistoricalView` is reached through `client.historical` and has no
    // constructor of its own. The flat-files surface is a hand-written
    // client-level member that lives on the unified `Client` itself, not on
    // its `client.historical` view, so it is mirrored onto `HistoricalClient`
    // directly and pinned against the unified `Client` in its own block
    // above. `close` is a client-lifecycle method (deterministic teardown)
    // that lives on the standalone `HistoricalClient` but not on the
    // `client.historical` sub-view — you close the owning `Client`, not its
    // view — so it is likewise excluded. Exclude all so this comparison pins
    // the generated data-fetch surface.
    const LIFECYCLE = new Set(['connect', 'connectFromFile', 'flatFileToPath', 'close']);
    // Every data-fetch method the MDDS-only client exposes must also exist
    // on the unified client's `client.historical` view — the historical
    // surface is generated identically onto both, so the MDDS set is a
    // strict subset of the `HistoricalView` set.
    for (const name of mdds) {
      if (LIFECYCLE.has(name)) continue;
      assert.ok(
        historical.has(name),
        `HistoricalClient method ${name} is missing from HistoricalView — the two historical surfaces have drifted`
      );
    }
  });
});

describe('HistoricalClient exposes the flat-files surface its contract promises', () => {
  // The class docstring states the historical / list / snapshot / at-time /
  // flat-files surface is identical to the unified client. The flat-file
  // entry points must therefore be reachable here, matching the unified
  // `Client` and the Python `HistoricalClient`, which delegates to its
  // wrapped client. A historical-only handle opens the same data channel,
  // so the namespace and the to-path writer are client-agnostic.
  const FLATFILE_GETTER = 'flatFiles';
  const FLATFILE_METHOD = 'flatFileToPath';

  it('declares the flatFiles getter in index.d.ts', () => {
    assert.match(
      mddsBlock[0],
      /get flatFiles\(\): FlatFilesNamespace/,
      'HistoricalClient must declare the flatFiles getter in index.d.ts'
    );
  });

  it('declares the flatFileToPath method in index.d.ts', () => {
    const methods = methodNames(mddsBlock[0]);
    assert.ok(
      methods.has(FLATFILE_METHOD),
      'HistoricalClient must declare flatFileToPath in index.d.ts'
    );
  });

  it('exposes both flat-file members on the loaded addon prototype', () => {
    const proto = mod.HistoricalClient.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(proto, FLATFILE_GETTER);
    assert.equal(
      typeof descriptor?.get,
      'function',
      'HistoricalClient.flatFiles must be an accessor on the prototype'
    );
    assert.equal(
      typeof proto[FLATFILE_METHOD],
      'function',
      'HistoricalClient.flatFileToPath must be a method on the prototype'
    );
  });

  it('matches the unified Client flat-file surface (control)', () => {
    const clientBlock = dts.match(/export declare class Client \{[\s\S]*?\n\}/);
    assert.match(
      clientBlock[0],
      /get flatFiles\(\): FlatFilesNamespace/,
      'the unified Client must retain the flatFiles getter'
    );
    assert.ok(
      methodNames(clientBlock[0]).has(FLATFILE_METHOD),
      'the unified Client must retain flatFileToPath'
    );
  });
});

describe('HistoricalClient never exposes the FPSS / streaming surface', () => {
  // The streaming, subscription, lifecycle, and ring-telemetry methods
  // live only on the unified client. An MDDS-only handle that surfaced
  // any of these could open or observe an FPSS slot, defeating the split.
  // This is the TypeScript parity of the Python `HistoricalClient` block-list
  // (`FPSS_TOUCHING_METHODS` in `thetadatadx-py/src/mdds_client.rs`).
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
        `HistoricalClient must NOT expose the FPSS-touching method ${banned}`
      );
    }
  });

  it('the unified client retains those methods (control)', () => {
    // Guard the guard: confirm the banned names are real methods on the
    // unified client's `client.stream` view, so the absence check above is
    // meaningful and not passing on a renamed/removed method.
    const stream = methodNames(streamViewBlock[0]);
    for (const present of ['startStreaming', 'subscribe', 'ringOccupancy']) {
      assert.ok(
        stream.has(present),
        `StreamView should still expose ${present}`
      );
    }
  });
});
