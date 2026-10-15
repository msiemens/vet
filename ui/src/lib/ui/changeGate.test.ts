import assert from "node:assert/strict";
import { test } from "node:test";
import { changeGate } from "./changeGate.ts";

void test("the first call is a change", () => {
  const changed = changeGate();
  assert.equal(changed("a"), true);
});

void test("the same values are not a change again", () => {
  const changed = changeGate();
  changed("a", 1);
  assert.equal(changed("a", 1), false);
});

void test("any one of the values differing is a change", () => {
  const changed = changeGate();
  changed("a", 1);
  assert.equal(changed("a", 2), true);
  assert.equal(changed("b", 2), true);
});

void test("a different number of values is a change", () => {
  const changed = changeGate();
  changed("a");
  assert.equal(changed("a", undefined), true);
});

void test("values are compared by reference, not by content", () => {
  const changed = changeGate();
  changed([1, 2]);
  assert.equal(changed([1, 2]), true);
  const same = [1, 2];
  changed(same);
  assert.equal(changed(same), false);
});
