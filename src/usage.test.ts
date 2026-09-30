import { test } from "node:test";
import assert from "node:assert/strict";
import { addUsage, compact, describeCost, describeUsage, runUsage, sessionUsage } from "./usage.ts";

test("numbers are compact", () => {
  assert.deepEqual([0, 950, 1234, 12_000, 3_400_000].map(compact), ["0", "950", "1.2k", "12k", "3.4M"]);
});

test("a Run's input already includes cached tokens", () => {
  const u = runUsage({ input_tokens: 2000, output_tokens: 150, total_tokens: 2150, cache_read_tokens: 1500, cache_write_tokens: 0 });
  assert.deepEqual(u, { input: 2000, output: 150, cached: 1500 });
  assert.equal(describeUsage(u!), "2k in · 150 out · 75% cached");
  assert.equal(runUsage({}), null); // a Run that reported no usage
  assert.equal(runUsage(undefined), null);
});

test("a Session record's input excludes cached tokens", () => {
  const s = { input_tokens: 500, output_tokens: 300, cache_read_tokens: 1500, cache_write_tokens: 0 };
  assert.equal(describeUsage(sessionUsage(s)!), "2k in · 300 out · 75% cached");
  assert.equal(sessionUsage({ input_tokens: 0, output_tokens: null }), null);
});

test("totals add up", () => {
  assert.deepEqual(addUsage(addUsage(null, { input: 10, output: 2, cached: 5 }), { input: 30, output: 4, cached: 25 }), { input: 40, output: 6, cached: 30 });
});

test("cost is Hermes's estimate, and $0 means Hermes has no price for the model", () => {
  assert.equal(describeCost(0.01234), "≈ $0.0123");
  assert.equal(describeCost(2.5), "≈ $2.50");
  assert.equal(describeCost(0), null);
  assert.equal(describeCost(null), null); // a Session with no Turns yet
});
