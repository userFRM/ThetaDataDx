// Structural test: every `StreamControl::*` Rust variant has a typed
// `#[napi(object)]` mirror exported from the addon, the discriminated-
// union `StreamEvent.kind` literal union covers every variant, and
// per-variant payload field names line up with the schema.
//
// We can't construct an `StreamEvent` without a live FPSS handshake
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
// `thetadatadx-rs/fpss_event_schema.toml`. Drift = test failure.
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
  // Named `ParseError` so the SDK ships no interface that shadows the
  // JS global `Error` class.
  { name: 'ParseError',         kind: 'parse_error',          payload: 'parseError',         fields: ['message'] },
  { name: 'UnknownFrame',       kind: 'unknown_frame',        payload: 'unknownFrame',       fields: ['code', 'payload'] },
  { name: 'Connected',          kind: 'connected',            payload: 'connected',          fields: [] },
  { name: 'Ping',               kind: 'ping',                 payload: 'ping',               fields: ['payload'] },
  { name: 'ReconnectedServer',  kind: 'reconnected_server',   payload: 'reconnectedServer',  fields: [] },
  { name: 'Restart',            kind: 'restart',              payload: 'restart',            fields: [] },
  { name: 'UnknownControl',     kind: 'unknown_control',      payload: 'unknownControl',     fields: [] },
];

describe('StreamControl typed variants', () => {
  it('every control kind is in the StreamEvent.kind literal union', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    const fpssEventMatch = dts.match(/export interface StreamEvent\s*\{[^}]+\}/s);
    assert.ok(fpssEventMatch, 'StreamEvent interface not found in index.d.ts');
    const fpssEventBlock = fpssEventMatch[0];
    for (const { kind } of CONTROL_VARIANTS) {
      assert.ok(
        fpssEventBlock.includes(`'${kind}'`),
        `StreamEvent.kind literal union missing '${kind}'`
      );
    }
  });

  it('every control variant has a typed payload field on StreamEvent', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    const fpssEventMatch = dts.match(/export interface StreamEvent\s*\{[^}]+\}/s);
    const fpssEventBlock = fpssEventMatch[0];
    for (const { name, payload } of CONTROL_VARIANTS) {
      // Match `payload?: TypeName` -- napi-rs lowers `Option<T>` to
      // `field?: T`. We tolerate any whitespace between the field
      // name and the type annotation.
      const re = new RegExp(`${payload}\\?\\s*:\\s*${name}\\b`);
      assert.match(
        fpssEventBlock,
        re,
        `StreamEvent.${payload}?: ${name} missing from interface`
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

  it("'simple' kind is gone from the StreamEvent.kind literal union", () => {
    const dts = readFileSync(dtsPath, 'utf8');
    const fpssEventMatch = dts.match(/export interface StreamEvent\s*\{[^}]+\}/s);
    const fpssEventBlock = fpssEventMatch[0];
    // The new union must not contain bare `'simple'` (use word
    // boundaries because `simple` substrings might still appear in
    // doc comments inside the interface).
    assert.equal(
      /\B'simple'\B/.test(fpssEventBlock) || /\b'simple'\b/.test(fpssEventBlock),
      false,
      `StreamEvent.kind union still contains 'simple'; ${fpssEventBlock}`
    );
  });
});

describe('StreamData typed variants', () => {
  // Every `kind = "data"` schema variant. napi-rs lowers the Rust
  // `contract_id: i32` field to `contractId: number` on each interface.
  const DATA_VARIANTS = ['Quote', 'Trade', 'OpenInterest', 'Ohlcvc', 'MarketValue'];

  it('every data event interface carries a contractId: number join key', () => {
    const dts = readFileSync(dtsPath, 'utf8');
    for (const name of DATA_VARIANTS) {
      const interfaceRe = new RegExp(`export interface ${name}\\s*\\{[^}]*\\}`, 's');
      const match = dts.match(interfaceRe);
      assert.ok(match, `interface ${name} missing from index.d.ts`);
      // napi lowers `i32` to `number`; snake_case `contract_id` to camel
      // `contractId`. The field is non-optional (present on every tick).
      assert.match(
        match[0],
        /\bcontractId\s*:\s*number\b/,
        `interface ${name} missing 'contractId: number' join key`
      );
    }
  });
});
