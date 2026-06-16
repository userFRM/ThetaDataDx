import { test } from "node:test";
import assert from "node:assert/strict";
import { Config } from "../index.js";

test("default wait strategy is low_latency", () => {
  const cfg = Config.production();
  assert.equal(cfg.waitStrategy, "low_latency");
});

for (const value of ["low_latency", "balanced", "efficient", "busy_spin"]) {
  test(`setWaitStrategy ${value} round-trips`, () => {
    const cfg = Config.production();
    cfg.setWaitStrategy(value);
    assert.equal(cfg.waitStrategy, value);
  });
}

test("setWaitStrategy case insensitive", () => {
  const cfg = Config.production();
  cfg.setWaitStrategy("BUSY_SPIN");
  assert.equal(cfg.waitStrategy, "busy_spin");
});

test("setWaitStrategy invalid throws", () => {
  const cfg = Config.production();
  assert.throws(() => cfg.setWaitStrategy("spin_forever"), /low_latency.*busy_spin/);
});

test("default wait tuning", () => {
  const cfg = Config.production();
  assert.equal(cfg.waitSpinIters, 100);
  assert.equal(cfg.waitYieldIters, 10);
  assert.equal(cfg.waitParkUs, 50);
});

test("wait tuning round-trips", () => {
  const cfg = Config.production();
  cfg.setWaitSpinIters(16);
  cfg.setWaitYieldIters(2);
  cfg.setWaitParkUs(200);
  assert.equal(cfg.waitSpinIters, 16);
  assert.equal(cfg.waitYieldIters, 2);
  assert.equal(cfg.waitParkUs, 200);
});

test("default consumer cpu is null", () => {
  const cfg = Config.production();
  assert.equal(cfg.consumerCpu, null);
});

test("consumer cpu round-trips", () => {
  const cfg = Config.production();
  cfg.setConsumerCpu(3);
  assert.equal(cfg.consumerCpu, 3);
  cfg.setConsumerCpu(null);
  assert.equal(cfg.consumerCpu, null);
});
