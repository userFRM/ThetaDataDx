// Structural test: every `FpssControl::*` Rust variant has a typed
// `#[napi(object)]` mirror exported from the addon, the discriminated-
// union `FpssEvent.kind` literal union covers every variant, and
// per-variant payload field names line up with the schema.
//
// We can't construct an `FpssEvent` without a live FPSS handshake
// (the typed payloads flow Rust -> JS only via `startStreaming`),
// so this test asserts the type-surface shape via `index.d.ts` and
// the JS module's exported class set rather than runtime values.

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const dtsPath = resolve(__dirname, '..', 'index.d.ts');

// Mirrors the `[events.<Variant>]` `kind = "control"` sections in
// `crates/thetadatadx/fpss_event_schema.toml`. Drift = test failure.
const CONTROL_VARIANTS = [
  { name: 'LoginSuccess',       kind: 'login_success',        payload: 'loginSuccess',       fields: ['permissions'] },
  { name: 'ContractAssigned',   kind: 'contract_assigned',    payload: 'contractAssigned',   fields: ['id', 'contract'] },
  { name: 'ReqResponse',        kind: 'req_response',         payload: 'reqResponse',        fields: ['reqId', 'result'] },
  { name: 'MarketOpen',         kind: 'market_open',          payload: 'marketOpen',         fields: [] },
  { name: 'MarketClose',        kind: 'market_close',         payload: 'marketClose',        fields: [] },
  { name: 'ServerError',        kind: 'server_error',         payload: 'serverError',        fields: ['message'] },
  { name: 'Disconnected',       kind: 'disconnected',         payload: 'disconnected',       fields: ['reason'] },
  { name: 'Reconnecting',       kind: 'reconnecting',         payload: 'reconnecting',       fields: ['reason', 'attempt', 'delayMs'] },
  { name: 'Reconnected',        kind: 'reconnected',          payload: 'reconnected',        fields: [] },
  // `Error` collides with the global `Error` class -- napi-rs still
  // emits `interface Error { message: string }` in the SDK module
  // namespace because TS resolves named exports before global lookup.
  { name: 'Error',              kind: 'error',                payload: 'error',              fields: ['message'] },
  { name: 'UnknownFrame',       kind: 'unknown_frame',        payload: 'unknownFrame',       fields: ['code', 'payload'] },
  { name: 'Connected',          kind: 'connected',            payload: 'connected',          fields: [] },
  { name: 'Ping',               kind: 'ping',                 payload: 'ping',               fields: ['payload'] },
  { name: 'ReconnectedServer',  kind: 'reconnected_server',   payload: 'reconnectedServer',  fields: [] },
  { name: 'Restart',            kind: 'restart',              payload: 'restart',            fields: [] },
  { name: 'UnknownControl',     kind: 'unknown_control',      payload: 'unknownControl',     fields: [] },
];

describe('FpssControl typed variants', () => {
  it('every control kind is in the FpssEvent.kind literal union', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    const fpssEventMatch = dts.match(/export interface FpssEvent\s*\{[^}]+\}/s);
    assert.ok(fpssEventMatch, 'FpssEvent interface not found in index.d.ts');
    const fpssEventBlock = fpssEventMatch[0];
    for (const { kind } of CONTROL_VARIANTS) {
      assert.ok(
        fpssEventBlock.includes(`'${kind}'`),
        `FpssEvent.kind literal union missing '${kind}'`
      );
    }
  });

  it('every control variant has a typed payload field on FpssEvent', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    const fpssEventMatch = dts.match(/export interface FpssEvent\s*\{[^}]+\}/s);
    const fpssEventBlock = fpssEventMatch[0];
    for (const { name, payload } of CONTROL_VARIANTS) {
      // Match `payload?: TypeName` -- napi-rs lowers `Option<T>` to
      // `field?: T`. We tolerate any whitespace between the field
      // name and the type annotation.
      const re = new RegExp(`${payload}\\?\\s*:\\s*${name}\\b`);
      assert.match(
        fpssEventBlock,
        re,
        `FpssEvent.${payload}?: ${name} missing from interface`
      );
    }
  });

  it('every control variant has its own typed interface in index.d.ts', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    for (const { name, fields } of CONTROL_VARIANTS) {
      // napi-rs emits `export interface Name { ... }` for each
      // `#[napi(object)]` struct (including unit-style empty ones).
      const interfaceRe = new RegExp(`export interface ${name}\\s*\\{[^}]*\\}`, 's');
      const match = dts.match(interfaceRe);
      assert.ok(match, `interface ${name} missing from index.d.ts`);
      const block = match[0];
      for (const field of fields) {
        // Loose match because some fields use `bigint` / `Buffer`
        // / nullable / nested struct types -- we only verify the
        // field name appears in the interface body.
        const fieldRe = new RegExp(`\\b${field}\\??\\s*:`);
        assert.match(
          block,
          fieldRe,
          `interface ${name} missing field '${field}'`
        );
      }
    }
  });

  it('FpssSimplePayload is gone (typed variants replace it)', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    assert.equal(
      dts.includes('FpssSimplePayload'),
      false,
      'FpssSimplePayload should have been removed; typed control ' +
      'variants (LoginSuccess, Disconnected, ...) are the SSOT now'
    );
  });

  it("'simple' kind is gone from the FpssEvent.kind literal union", () => {
    const dts = readFileSync(dtsPath, 'utf8');
    const fpssEventMatch = dts.match(/export interface FpssEvent\s*\{[^}]+\}/s);
    const fpssEventBlock = fpssEventMatch[0];
    // The new union must not contain bare `'simple'` (use word
    // boundaries because `simple` substrings might still appear in
    // doc comments inside the interface).
    assert.equal(
      /\B'simple'\B/.test(fpssEventBlock) || /\b'simple'\b/.test(fpssEventBlock),
      false,
      `FpssEvent.kind union still contains 'simple'; ${fpssEventBlock}`
    );
  });
});
