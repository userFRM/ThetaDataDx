// Standalone `StreamingClient` structural contract (offline — no connect).
//
// `StreamingClient` is the streaming-only napi handle over
// `thetadatadx::fpss::StreamingClient`: the FPSS TLS transport with no MDDS /
// Nexus historical surface. It mirrors the Python `StreamingClient`
// (`sdks/python/src/fpss_client.rs`), the C++ `thetadatadx::StreamingClient`, and the
// C ABI `thetadatadx_fpss_*` entry points. These assertions pin the split
// structurally against `index.d.ts` and the loaded addon so a change that
// drops the streaming surface — or leaks a historical method onto it —
// fails here without live credentials.

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

const fpssBlock = dts.match(/export declare class StreamingClient \{[\s\S]*?\n\}/);

function methodNames(block) {
  return new Set(
    block
      .split('\n')
      .map((line) => line.match(/^\s+(?:static\s+)?([a-z][a-zA-Z0-9]*)\(/))
      .filter(Boolean)
      .map((m) => m[1])
  );
}

describe('StreamingClient native addon surface', () => {
  it('exports the StreamingClient class with the creds-first connect factories', () => {
    assert.ok(mod.StreamingClient, 'StreamingClient should be exported');
    for (const factory of ['connect', 'connectFromFile']) {
      assert.equal(
        typeof mod.StreamingClient[factory],
        'function',
        `StreamingClient.${factory} should be a static factory`
      );
    }
  });

  it('declares the StreamingClient class in index.d.ts', () => {
    assert.ok(fpssBlock, 'StreamingClient class missing from index.d.ts');
  });
});

describe('StreamingClient carries the full streaming surface', () => {
  // The streaming, subscription, lifecycle, and ring-telemetry surface
  // that the Python `StreamingClient` exposes (the parity rows flipped to
  // `typescript = true` in sdks/parity.toml). Pin the whole set so a
  // regression that drops any one of them fails here.
  const STREAMING_SURFACE = [
    'startStreaming',
    'stopStreaming',
    'shutdown',
    'reconnect',
    'isStreaming',
    'isAuthenticated',
    'awaitDrain',
    'subscribe',
    'subscribeMany',
    'unsubscribe',
    'unsubscribeMany',
    'activeSubscriptions',
    'activeFullSubscriptions',
    'droppedEventCount',
    'ringOccupancy',
    'ringCapacity',
    'panicCount',
    'slowCallbackCount',
    'setSlowCallbackThresholdUs',
    'millisSinceLastEvent',
    'lastEventReceivedAtUnixNanos',
    'lastConnectedAddr',
  ];

  it('declares every streaming / lifecycle / telemetry method', () => {
    const methods = methodNames(fpssBlock[0]);
    for (const expected of STREAMING_SURFACE) {
      assert.ok(
        methods.has(expected),
        `StreamingClient must expose ${expected}`
      );
    }
  });

  it('the network-bound lifecycle methods are async so they never block the event loop', () => {
    assert.match(
      fpssBlock[0],
      /awaitDrain\(timeoutMs: number\): Promise<boolean>/,
      'awaitDrain must resolve Promise<boolean> so it does not block the event loop'
    );
    // The FPSS connect plus authentication handshake runs inside
    // startStreaming and reconnect, so both resolve a Promise rather than
    // freezing the libuv thread for the handshake.
    assert.match(
      fpssBlock[0],
      /startStreaming\(callback:[\s\S]*?\): Promise<void>/,
      'startStreaming must resolve Promise<void> so the connect handshake does not block the event loop'
    );
    assert.match(
      fpssBlock[0],
      /reconnect\(\): Promise<void>/,
      'reconnect must resolve Promise<void> so the connect handshake does not block the event loop'
    );
  });
});

describe('StreamingClient never exposes the MDDS / historical surface', () => {
  // FPSS-only: no historical / list / snapshot / at-time / calendar
  // method may appear. An FPSS client that surfaced these would imply an
  // MDDS channel it never opens. This is the inverse of the HistoricalClient
  // FPSS-free guard — together they pin the two standalone surfaces apart.
  it('declares no historical data-fetch families', () => {
    const familyRe =
      /^\s+(?:static\s+)?((?:stock|option|index)History\w*|\w*Snapshot\w*|\w*AtTime\w*|calendarOnDate|calendarYear|interestRateHistory\w*|listRoots|optionListContracts)\(/;
    const leaked = fpssBlock[0]
      .split('\n')
      .filter((line) => familyRe.test(line))
      .map((l) => l.trim());
    assert.deepEqual(
      leaked,
      [],
      `StreamingClient must not expose historical methods; found: ${leaked.join(', ')}`
    );
  });
});
