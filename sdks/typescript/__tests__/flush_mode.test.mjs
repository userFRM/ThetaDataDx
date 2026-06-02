import { test } from "node:test";
import assert from "node:assert/strict";
import { Config } from "../index.js";

test("default flush_mode is batched", () => {
  const cfg = Config.production();
  assert.equal(cfg.flushMode, "batched");
});

test("setFlushMode immediate round-trips", () => {
  const cfg = Config.production();
  cfg.setFlushMode("immediate");
  assert.equal(cfg.flushMode, "immediate");
});

test("setFlushMode batched round-trips", () => {
  const cfg = Config.production();
  cfg.setFlushMode("immediate");
  cfg.setFlushMode("batched");
  assert.equal(cfg.flushMode, "batched");
});

test("setFlushMode case insensitive", () => {
  const cfg = Config.production();
  cfg.setFlushMode("IMMEDIATE");
  assert.equal(cfg.flushMode, "immediate");
});

test("setFlushMode invalid throws", () => {
  const cfg = Config.production();
  assert.throws(() => cfg.setFlushMode("instant"), /batched.*immediate/);
});
