import assert from "node:assert/strict";
import { test } from "node:test";
import type { Column } from "./columns.ts";
import { parseJson } from "../storage.ts";
import { type Sort, nextSort, parseSort, serializeSort, sortRows } from "./sorting.ts";

interface Row {
  name: string;
  at?: number;
}

const NAME: Column<Row> = { id: "name", label: "Name", sortBy: (r) => r.name };
const AT: Column<Row> = { id: "at", label: "At", sortBy: (r) => r.at, firstSort: "descending" };
const COLUMNS: Column<Row>[] = [NAME, AT, { id: "note", label: "Note" }];

const names = (rows: readonly Row[]) => rows.map((r) => r.name);

void test("clicks on a header sort by it, then the other way, then back to the default order", () => {
  const first = nextSort(undefined, NAME);
  assert.deepEqual(first, { column: "name", direction: "ascending" });
  const second = nextSort(first, NAME);
  assert.deepEqual(second, { column: "name", direction: "descending" });
  assert.equal(nextSort(second, NAME), undefined);
});

void test("a column may sort the other way first", () => {
  const first = nextSort(undefined, AT);
  assert.deepEqual(first, { column: "at", direction: "descending" });
  assert.deepEqual(nextSort(first, AT), { column: "at", direction: "ascending" });
  assert.equal(nextSort(nextSort(first, AT), AT), undefined);
});

void test("a click on another column sorts by it from its first direction", () => {
  assert.deepEqual(nextSort({ column: "name", direction: "descending" }, AT), {
    column: "at",
    direction: "descending",
  });
});

void test("names sort by the locale, ignoring case and with numbers in order", () => {
  const rows = [{ name: "b" }, { name: "Ä" }, { name: "v10" }, { name: "a" }, { name: "v2" }];
  const sort: Sort = { column: "name", direction: "ascending" };
  assert.deepEqual(names(sortRows(rows, COLUMNS, sort)), ["a", "Ä", "b", "v2", "v10"]);
  assert.deepEqual(names(sortRows(rows, COLUMNS, { ...sort, direction: "descending" })), ["v10", "v2", "b", "Ä", "a"]);
});

void test("rows without a value come last in either direction", () => {
  const rows = [{ name: "none" }, { name: "old", at: 1 }, { name: "new", at: 5 }];
  assert.deepEqual(names(sortRows(rows, COLUMNS, { column: "at", direction: "descending" })), ["new", "old", "none"]);
  assert.deepEqual(names(sortRows(rows, COLUMNS, { column: "at", direction: "ascending" })), ["old", "new", "none"]);
});

void test("rows that sort alike keep the default order between them", () => {
  const rows = [{ name: "x", at: 2 }, { name: "y", at: 1 }, { name: "z", at: 2 }, { name: "u" }, { name: "w" }];
  for (const direction of ["ascending", "descending"] as const) {
    const sorted = names(sortRows(rows, COLUMNS, { column: "at", direction }));
    assert.ok(sorted.indexOf("x") < sorted.indexOf("z"), direction);
    assert.deepEqual(sorted.slice(3), ["u", "w"], direction);
  }
});

void test("without a sort, or by a column that cannot sort, the rows keep their order", () => {
  const rows = [{ name: "b" }, { name: "a" }];
  assert.deepEqual(names(sortRows(rows, COLUMNS, undefined)), ["b", "a"]);
  assert.deepEqual(names(sortRows(rows, COLUMNS, { column: "note", direction: "ascending" })), ["b", "a"]);
});

void test("a stored sort comes back as it was written, and the default order stores nothing", () => {
  const sort: Sort = { column: "at", direction: "ascending" };
  assert.deepEqual(parseSort(COLUMNS, parseJson(serializeSort(sort))), sort);
  assert.equal(serializeSort(undefined), undefined);
  assert.equal(parseSort(COLUMNS, undefined), undefined);
});

void test("a stored sort by a column that is gone or cannot sort, or that is not a sort, gives the default order", () => {
  const texts = [
    JSON.stringify({ column: "gone", direction: "ascending" }),
    JSON.stringify({ column: "note", direction: "ascending" }),
    JSON.stringify({ column: "name", direction: "up" }),
    "",
    "{",
    "null",
    "[1]",
  ];
  for (const text of texts) assert.equal(parseSort(COLUMNS, parseJson(text)), undefined, text);
});
