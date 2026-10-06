import assert from "node:assert/strict";
import { test } from "node:test";
import { rowWindow, scrollToRow } from "./rowWindow.ts";

void test("the window covers the viewport plus overscan", () => {
  // 220 px of 22 px rows at the top: rows 0..10, plus 5 below.
  assert.deepEqual(rowWindow(0, 220, 22, 1000, 5), { start: 0, end: 15 });
  // Scrolled to row 100 exactly.
  assert.deepEqual(rowWindow(2200, 220, 22, 1000, 5), { start: 95, end: 115 });
});

void test("a partly visible row counts", () => {
  assert.deepEqual(rowWindow(11, 22, 22, 1000, 0), { start: 0, end: 2 });
});

void test("the window is clamped to the list", () => {
  assert.deepEqual(rowWindow(0, 220, 22, 3, 5), { start: 0, end: 3 });
  assert.deepEqual(rowWindow(1_000_000, 220, 22, 100, 5), { start: 100, end: 100 });
  assert.deepEqual(rowWindow(-50, 220, 22, 100, 0), { start: 0, end: 10 });
});

void test("an empty list or a zero height mounts nothing", () => {
  assert.deepEqual(rowWindow(0, 220, 22, 0, 5), { start: 0, end: 0 });
  assert.deepEqual(rowWindow(0, 220, 0, 10, 5), { start: 0, end: 0 });
});

void test("the window stays small for a huge list", () => {
  const { start, end } = rowWindow(220_000, 600, 22, 20_000, 8);
  assert.ok(end - start <= 600 / 22 + 1 + 16);
});

void test("scrollToRow moves only as far as needed", () => {
  assert.equal(scrollToRow(5, 0, 220, 22), 0); // visible
  assert.equal(scrollToRow(2, 100, 220, 22), 44); // above: row's top
  assert.equal(scrollToRow(20, 0, 220, 22), 242); // below: row's bottom at the viewport's bottom
});
