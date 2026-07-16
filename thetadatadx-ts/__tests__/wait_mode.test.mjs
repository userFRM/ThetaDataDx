import { test } from "node:test";
import assert from "node:assert/strict";
import { Config } from "../index.js";

test("defaults are spin and 1000us", () => {
  const cfg = Config.production();
  assert.equal(cfg.waitMode, "spin");
  assert.equal(cfg.parkIntervalUs, 1000n);
});

test("wait mode round-trips every variant", () => {
  const cfg = Config.production();
  for (const mode of ["busyspin", "park", "backoff", "spin"]) {
    cfg.setWaitMode(mode);
    assert.equal(cfg.waitMode, mode);
  }
});

test("wait mode is case-insensitive and normalises lowercase", () => {
  const cfg = Config.production();
  cfg.setWaitMode("BACKOFF");
  assert.equal(cfg.waitMode, "backoff");
});

test("wait mode rejects unknown", () => {
  const cfg = Config.production();
  assert.throws(() => cfg.setWaitMode("block"));
});

test("park interval us round-trips", () => {
  const cfg = Config.production();
  cfg.setParkIntervalUs(250n);
  assert.equal(cfg.parkIntervalUs, 250n);
});
