import assert from "node:assert/strict";
import { test } from "node:test";
import { WrapRows, rowColumns } from "./wrapRows.ts";

void test("a line takes one row for every full width of columns it fills", () => {
  const rows = new WrapRows(["short", "x".repeat(10), "x".repeat(11), "x".repeat(25), ""].join("\n"), 10);
  // Lines 0 to 4 take 1, 1, 2, 3 and 1 rows: the extra rows are 0, 0, 1, 2 and 0.
  assert.equal(rows.extra(0, 5), 3);
  assert.equal(rows.extra(2, 3), 1);
  assert.equal(rows.extra(3, 4), 2);
  assert.equal(rows.extra(0, 2), 0);
});

void test("lines break after spaces: a word that does not fit starts a new row, and one longer than a row breaks inside", () => {
  // 20 columns would fit two rows of 10, but each word is followed by one that does not fit.
  assert.equal(new WrapRows("aaaaaa bbbbbb cccccc", 10).extra(0, 1), 2);
  // The long word starts a row of its own, and fills three.
  assert.equal(new WrapRows(`ab ${"x".repeat(25)}`, 10).extra(0, 1), 3);
  assert.equal(new WrapRows("aaaa bbbb cc", 10).extra(0, 1), 1);
});

void test("a range counts only its own lines, and lines past the end have no extra rows", () => {
  const rows = new WrapRows(`${"x".repeat(30)}\nshort\n${"x".repeat(30)}`, 10);
  assert.equal(rows.extra(1, 3), 2);
  assert.equal(rows.extra(2, 50), 2);
  assert.equal(rows.extra(40, 50), 0);
});

void test("tabs advance to the next tab stop and a carriage return before a line break is not a column", () => {
  // A tab at column 0 is 4 columns wide, so "\tab" is 6 columns: two rows of 4.
  assert.equal(new WrapRows("\tab", 4).extra(0, 1), 1);
  assert.equal(new WrapRows("abcd\r\nef", 4).extra(0, 2), 0);
});

void test("a column that cannot hold one character still gives every line a finite height", () => {
  assert.equal(new WrapRows("abc", 1).extra(0, 1), 2);
});

void test("a side by side view wraps at half the width, and more digits take columns from the code", () => {
  const unified = rowColumns(1000, "unified", 7, 500);
  const split = rowColumns(1000, "split", 7, 500);
  assert.ok(split < unified && split > unified / 2 - 10, `${split} of ${unified}`);
  assert.ok(rowColumns(1000, "unified", 7, 5_000_000) < unified);
  assert.equal(rowColumns(20, "split", 7, 5), 1);
});
