import assert from "node:assert/strict";
import { test } from "node:test";
import { parseSeen, withSeen } from "./seenTimes.ts";

void test("the oldest times are dropped beyond the most that are kept", () => {
  let seen = {};
  for (const key of ["a", "b", "c"]) seen = withSeen(seen, key, 10, 2);
  assert.deepEqual(Object.keys(seen), ["b", "c"]);
});

void test("writing a pull request again makes it the newest and does not cost another's time", () => {
  let seen = withSeen(withSeen({}, "a", 1, 2), "b", 2, 2);
  seen = withSeen(seen, "a", 3, 2);
  assert.deepEqual(seen, { b: 2, a: 3 });
});

void test("what is kept but is not a time is dropped", () => {
  assert.deepEqual(parseSeen({ a: 5, b: "x", c: -1, d: null }), { a: 5 });
  assert.deepEqual(parseSeen([1]), {});
  assert.deepEqual(parseSeen(undefined), {});
});
