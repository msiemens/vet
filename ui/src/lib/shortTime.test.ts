import assert from "node:assert/strict";
import { test } from "node:test";
import { fullTime, shortTime } from "./shortTime.ts";

const now = new Date(2026, 9, 7, 15, 30).getTime();
const at = (...date: [number, number, number?, number?, number?]) => new Date(...date).getTime() / 1000;

void test("the label shortens with distance", () => {
  assert.equal(shortTime(now / 1000 - 20, now), "now");
  assert.equal(shortTime(at(2026, 9, 7, 9, 5), now), "9:05 AM");
  assert.equal(shortTime(at(2026, 9, 5, 23, 0), now), "Mon");
  assert.equal(shortTime(at(2026, 8, 28, 12), now), "Sep 28");
  assert.equal(shortTime(at(2025, 11, 31, 12), now), "Dec 31, 2025");
});

void test("the detailed style adds the time of day to the weekday and changes nothing else", () => {
  assert.equal(shortTime(now / 1000 - 20, now, "detailed"), "now");
  assert.equal(shortTime(at(2026, 9, 7, 9, 5), now, "detailed"), "9:05 AM");
  assert.equal(shortTime(at(2026, 9, 5, 23, 0), now, "detailed"), "Mon 11:00 PM");
  assert.equal(shortTime(at(2026, 8, 28, 12), now, "detailed"), "Sep 28");
});

void test("a time in the future is now", () => {
  assert.equal(shortTime(now / 1000 + 5 * 60, now), "now");
});

void test("the tooltip carries what the short time drops: weekday, year and the minute", () => {
  const text = fullTime(at(2026, 9, 5, 16, 30));
  assert.match(text, /2026/);
  assert.match(text, /30/);
  assert.match(text, /Mon/);
});
