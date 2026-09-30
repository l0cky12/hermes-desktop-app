import { test } from "node:test";
import assert from "node:assert/strict";
import { dollars, percentLeft, tokens } from "./usage-format.ts";

test("spend shows cents, and sub-dollar amounts to four places", () => {
  assert.equal(dollars(0), "$0.00");
  assert.equal(dollars(0.25), "$0.25");
  assert.equal(dollars(0.0042), "$0.0042");
  assert.equal(dollars(0.123456), "$0.1235");
  assert.equal(dollars(1234.5), "$1,234.50");
});

test("token counts are compact", () => {
  assert.equal(tokens(0), "0");
  assert.equal(tokens(950), "950");
  assert.equal(tokens(1234), "1.2k");
  assert.equal(tokens(3_400_000), "3.4M");
});

test("the share of a limit window left, from Hermes's used percent", () => {
  assert.equal(percentLeft(65.510374511), 34);
  assert.equal(percentLeft(0), 100);
  assert.equal(percentLeft(120), 0);
});
