import assert from "node:assert/strict";
import { test } from "node:test";
import { clock, collator, count, durationText, noun, plural } from "./format.ts";

void test("a count takes the singular for one and the plural for any other number", () => {
  assert.equal(plural(1, "file"), "1 file");
  assert.equal(plural(0, "file"), "0 files");
  assert.equal(plural(2, "reply", "replies"), "2 replies");
  assert.equal(noun(1, "reply", "replies"), "reply");
});

void test("counts carry thousands separators, whatever the system locale", () => {
  assert.equal(count(1234567), "1,234,567");
  assert.equal(plural(1500, "match", "matches"), "1,500 matches");
});

void test("the time of day is 24-hour, with seconds on request", () => {
  const at = new Date(2024, 0, 5, 0, 7, 9).getTime() / 1000;
  assert.equal(clock(at), "00:07");
  assert.equal(clock(at, true), "00:07:09");
  const afternoon = new Date(2024, 0, 5, 15, 30, 0).getTime() / 1000;
  assert.equal(clock(afternoon), "15:30");
});

void test("a collator orders as the options say", () => {
  assert.deepEqual(["a10", "a2"].sort(collator({ numeric: true }).compare), ["a2", "a10"]);
  assert.equal(collator({ sensitivity: "base" }).compare("Ab", "aB"), 0);
});

void test("durations read as the platform shows them", () => {
  assert.deepEqual([12, 65, 192, 280, 3600, 3725].map(durationText), [
    "12s",
    "1m 05s",
    "3m 12s",
    "4m 40s",
    "1h 00m",
    "1h 02m",
  ]);
});
