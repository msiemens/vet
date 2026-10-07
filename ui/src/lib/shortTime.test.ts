import assert from "node:assert/strict";
import { test } from "node:test";
import { shortTime } from "./shortTime.ts";

void test("the label shortens with distance", () => {
  const now = new Date(2026, 9, 7, 15, 30).getTime();
  const at = (...date: [number, number, number?, number?, number?]) => new Date(...date).getTime() / 1000;
  assert.equal(shortTime(now / 1000 - 20, now), "now");
  assert.equal(shortTime(at(2026, 9, 7, 9, 5), now), "9:05 AM");
  assert.equal(shortTime(at(2026, 9, 5, 23, 0), now), "Mon");
  assert.equal(shortTime(at(2026, 8, 28, 12), now), "Sep 28");
  assert.equal(shortTime(at(2025, 11, 31, 12), now), "Dec 31, 2025");
});
