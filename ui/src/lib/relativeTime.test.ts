import assert from "node:assert/strict";
import { test } from "node:test";
import { fullTime, relativeTime } from "./relativeTime.ts";

const now = Date.UTC(2026, 9, 12, 12, 0, 0);
const ago = (seconds: number) => now / 1000 - seconds;
const MIN = 60;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

void test("a comment's age is spelled in the largest whole unit", () => {
  assert.equal(relativeTime(ago(30), now), "now");
  assert.equal(relativeTime(ago(5 * MIN + 59), now), "5 min ago");
  assert.equal(relativeTime(ago(59 * MIN), now), "59 min ago");
  assert.equal(relativeTime(ago(HOUR), now), "1 h ago");
  assert.equal(relativeTime(ago(23 * HOUR + 59 * MIN), now), "23 h ago");
  assert.equal(relativeTime(ago(DAY), now), "yesterday");
  assert.equal(relativeTime(ago(3 * DAY), now), "3 days ago");
});

void test("a comment older than a week is dated, and the year shows only when it is not this one", () => {
  const week = relativeTime(ago(8 * DAY), now);
  assert.match(week, /^Oct \d+$/);
  assert.match(relativeTime(ago(400 * DAY), now), /\d{4}$/);
});

void test("a comment from the future is just now", () => {
  assert.equal(relativeTime(ago(-5 * MIN), now), "now");
});

void test("the tooltip carries what the relative time drops: weekday, year and the minute", () => {
  const at = new Date(2026, 9, 12, 16, 30).getTime() / 1000;
  const text = fullTime(at);
  assert.match(text, /2026/);
  assert.match(text, /30/);
  assert.notEqual(text, relativeTime(at));
});
