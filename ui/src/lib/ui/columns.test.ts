import assert from "node:assert/strict";
import { test } from "node:test";
import { type Column, gridTemplate, handleSide, parseWidths, resized, withWidth, withoutWidth } from "./columns.ts";

const COLUMNS: Column[] = [
  { id: "version", label: "Version", width: 56 },
  { id: "files", label: "Files", flex: true, min: 80 },
  { id: "ci", label: "", width: 14, fixed: true },
  { id: "lines", label: "Lines", width: 90, min: 50 },
];

void test("columns without a width set keep their defaults and the flexible one takes the rest", () => {
  assert.equal(gridTemplate(COLUMNS, {}), "56px minmax(0, 1fr) 14px 90px");
  assert.equal(gridTemplate(COLUMNS, { lines: 120 }), "56px minmax(0, 1fr) 14px 120px");
});

void test("a reset column goes back to its default and the others keep theirs", () => {
  const widths = withWidth(withWidth({}, "version", 80), "lines", 120);
  assert.equal(gridTemplate(COLUMNS, withoutWidth(widths, "lines")), "80px minmax(0, 1fr) 14px 90px");
});

void test("a column is resized by the edge that faces the flexible column", () => {
  assert.equal(handleSide(COLUMNS, 0), "end");
  assert.equal(handleSide(COLUMNS, 3), "start");
});

void test("neither the flexible column nor a fixed one can be resized", () => {
  assert.equal(handleSide(COLUMNS, 1), undefined);
  assert.equal(handleSide(COLUMNS, 2), undefined);
});

void test("without a flexible column every resizable column has its right edge", () => {
  const columns: Column[] = [
    { id: "a", label: "A", width: 50 },
    { id: "b", label: "B", width: 50 },
  ];
  assert.equal(handleSide(columns, 1), "end");
});

void test("a column grows as its edge moves away from it", () => {
  assert.equal(resized(COLUMNS[0], "end", 56, 30), 86);
  assert.equal(resized(COLUMNS[3], "start", 90, -30), 120);
  assert.equal(resized(COLUMNS[3], "start", 90, 20), 70);
});

void test("a column is never narrower than its minimum", () => {
  assert.equal(resized(COLUMNS[3], "start", 90, 200), 50);
  assert.equal(resized(COLUMNS[0], "end", 56, -100), 40);
});

void test("a column takes no more than the flexible column can give", () => {
  assert.equal(resized(COLUMNS[0], "end", 56, 500, 100), 156);
  // The flexible column is already below its minimum: the column may only shrink.
  assert.equal(resized(COLUMNS[0], "end", 56, 30, -10), 56);
  assert.equal(resized(COLUMNS[0], "end", 56, -10, -10), 46);
});

void test("stored widths come back as they were written", () => {
  const widths = withWidth(withWidth({}, "version", 72), "lines", 130);
  assert.deepEqual(parseWidths(COLUMNS, JSON.parse(JSON.stringify(widths)) as unknown), widths);
});

void test("stored widths keep only the resizable columns, at least as wide as their minimum", () => {
  const stored = { version: 10, files: 300, ci: 40, lines: "wide", gone: 100 };
  assert.deepEqual(parseWidths(COLUMNS, stored), { version: 40 });
});

void test("a value that is not stored widths gives the defaults", () => {
  for (const value of ["", null, 3, [90]]) {
    assert.deepEqual(parseWidths(COLUMNS, value), {}, JSON.stringify(value));
  }
  assert.deepEqual(parseWidths(COLUMNS, undefined), {});
});
