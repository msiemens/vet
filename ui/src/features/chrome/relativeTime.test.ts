import assert from "node:assert/strict";
import { test } from "node:test";
import { relativeTime } from "./relativeTime.ts";

const now = Date.UTC(2026, 9, 6) / 1000;
const ago = (seconds: number) => relativeTime(now - seconds, now * 1000);

void test("picks the largest unit that fits", () => {
  assert.equal(ago(10), "just now");
  assert.equal(ago(5 * 60), "5 min. ago");
  assert.equal(ago(3 * 3600), "3 hr. ago");
  assert.equal(ago(2 * 86400), "2 days ago");
  assert.equal(ago(21 * 86400), "3 wk. ago");
  assert.equal(ago(400 * 86400), "1 yr. ago");
});
