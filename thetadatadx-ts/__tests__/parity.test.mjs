// The built package against the cross-binding manifest `parity.json`.
//
// The manifest names the hand-written surface the Python, TypeScript and C++
// bindings share. This holds the TypeScript column of it, both ways: every
// listed class, member and function exists on the package entry, and a listed
// class has no public member the manifest does not name. A name exported only
// as a type is looked up in the published declarations.

import { it } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { createRequire } from 'node:module';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..');
const root = resolve(pkg, '..');
const sdk = createRequire(import.meta.url)(pkg);
const manifest = JSON.parse(readFileSync(resolve(root, 'parity.json'), 'utf8'));
const declarations = ['index.d.ts', 'streaming-session.d.ts']
  .map((f) => readFileSync(resolve(pkg, f), 'utf8'))
  .join('\n');
// The config table writes the remaining `Config` members.
const configTable = [
  ...readFileSync(resolve(root, 'thetadatadx-rs/src/surface/config.rs'), 'utf8').matchAll(
    /^\s*(set|get) thetadatadx_config_(?:set|get)_(\w+)\(/gm,
  ),
].map(([, dir, name]) => tsName(dir === 'set' ? `set_${name}` : name, '*'));

function camel(snake) {
  return snake.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());
}

// The TypeScript spelling of a manifest entry, or undefined when TypeScript lacks it.
function tsName(name, spec) {
  if (typeof spec === 'object') return spec.typescript;
  if (spec !== '*' && !spec.split(' ').includes('typescript')) return undefined;
  return camel(name.replace(/^get_/, ''));
}

// Own names along a prototype chain, stopping at the language's built-ins,
// less the ones every function or object carries.
const builtins = new Set([Object.prototype, Function.prototype, Error.prototype, Error]);
function names(start, ignore) {
  const out = new Set();
  for (let o = start; o && !builtins.has(o); o = Object.getPrototypeOf(o)) {
    for (const n of Object.getOwnPropertyNames(o)) if (!ignore.includes(n)) out.add(n);
  }
  return out;
}
// Node 20 gives native constructors their own `arguments` and `caller`.
const statics = (cls) => names(cls, ['length', 'name', 'prototype', 'arguments', 'caller']);
const instance = (cls) => names(cls.prototype, ['constructor', 'toString']);

it('classes and members match parity.json', () => {
  const wrong = [];
  for (const [name, entry] of Object.entries(manifest.classes)) {
    const tsClass = entry.typescript === undefined ? name : entry.typescript;
    if (tsClass === null) {
      if (name in sdk) wrong.push(`${name}: exported, but the manifest lists no TypeScript class`);
      continue;
    }
    const cls = sdk[tsClass];
    if (cls === undefined) {
      if (!new RegExp(`^export (declare )?(class|interface|type|const) ${tsClass}\\b`, 'm').test(declarations)) {
        wrong.push(`${name}: \`${tsClass}\` is neither exported nor declared`);
      }
      continue;
    }
    const actual = new Set([...statics(cls), ...instance(cls)]);
    const expected = new Set();
    for (const [member, spec] of Object.entries(entry.members)) {
      const n = tsName(member, spec);
      if (n === undefined) continue;
      expected.add(n);
      if (!actual.has(n)) wrong.push(`${tsClass}.${n}: missing`);
    }
    const open = entry.open === '*' || (entry.open ?? '').split(' ').includes('typescript');
    if (open) continue;
    if (cls === sdk.Config) configTable.forEach((n) => expected.add(n));
    for (const n of actual) if (!expected.has(n)) wrong.push(`${tsClass}.${n}: not in parity.json`);
  }
  assert.deepEqual(wrong, []);
});

it('functions match parity.json', () => {
  const wrong = [];
  const listed = new Set();
  for (const [name, spec] of Object.entries(manifest.functions)) {
    const n = tsName(name, spec);
    if (n === undefined) continue;
    listed.add(n);
    if (typeof sdk.Util[n] !== 'function') wrong.push(`Util.${n}: missing`);
  }
  for (const n of statics(sdk.Util)) if (!listed.has(n)) wrong.push(`Util.${n}: not in parity.json`);
  assert.deepEqual(wrong, []);
});

it('subscription kinds match parity.json', () => {
  const contract = sdk.Contract.stock('AAPL');
  const secType = sdk.SecType.option();
  const kinds = [
    ...['quote', 'trade', 'openInterest', 'marketValue'].map((k) => contract[k]().kind),
    ...['fullTrades', 'fullOpenInterest'].map((k) => secType[k]().kind),
  ];
  assert.deepEqual(kinds, manifest.subscription_kinds);
});

it('no public name embeds an implementation detail', () => {
  const all = new Set(Object.keys(sdk));
  for (const value of Object.values(sdk)) {
    if (typeof value !== 'function') continue;
    for (const n of statics(value)) all.add(n);
    if (value.prototype) for (const n of instance(value)) all.add(n);
  }
  const leaked = [...all].filter((n) =>
    manifest.banned_identifier_tokens.some((token) => n.toLowerCase().includes(token)),
  );
  assert.deepEqual(leaked, []);
});
