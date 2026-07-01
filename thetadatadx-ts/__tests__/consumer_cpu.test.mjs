import { test } from "node:test";
import assert from "node:assert/strict";
import { Config } from "../index.js";

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
