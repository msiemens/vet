import assert from "node:assert/strict";
import { test } from "node:test";
import { arrowKey, toggleIn, toggled } from "./foldKeys.ts";

interface Row {
  kind: string;
  depth: number;
  open: boolean;
  holds: boolean;
}
const tree: Row[] = [
  { kind: "file", depth: 1, open: true, holds: true },
  { kind: "symbol", depth: 2, open: false, holds: true },
  { kind: "symbol", depth: 3, open: false, holds: false },
  { kind: "file", depth: 1, open: false, holds: true },
  { kind: "file", depth: 1, open: false, holds: false },
];
const folding = {
  holds: (r: Row) => r.holds,
  open: (r: Row) => r.open,
  leaves: (r: Row) => r.kind === "symbol",
};

void test("→ opens a shut row, then moves into it", () => {
  assert.deepEqual(arrowKey(tree, 1, "right", folding), { toggle: tree[1] });
  assert.deepEqual(arrowKey(tree, 0, "right", folding), { move: 1 });
  assert.equal(arrowKey(tree, 2, "right", folding), undefined);
  assert.deepEqual(arrowKey(tree, 3, "right", folding), { toggle: tree[3] });
});

void test("← shuts an open row, else moves a row that leaves to the row that holds it", () => {
  assert.deepEqual(arrowKey(tree, 0, "left", folding), { toggle: tree[0] });
  assert.deepEqual(arrowKey(tree, 2, "left", folding), { move: 1 });
  assert.deepEqual(arrowKey(tree, 1, "left", folding), { move: 0 });
  assert.equal(arrowKey(tree, 3, "left", folding), undefined);
});

void test("← from a row skips its siblings to the row that holds it", () => {
  const rows: Row[] = [
    { kind: "file", depth: 1, open: true, holds: true },
    { kind: "symbol", depth: 2, open: false, holds: false },
    { kind: "symbol", depth: 2, open: false, holds: false },
  ];
  assert.deepEqual(arrowKey(rows, 2, "left", folding), { move: 0 });
});

void test("toggling a key puts it in a set that lacks it and takes it out of one that has it", () => {
  const set = new Set(["a"]);
  toggleIn(set, "b");
  assert.deepEqual([...set], ["a", "b"]);
  toggleIn(set, "a");
  assert.deepEqual([...set], ["b"]);
});

void test("a toggled copy leaves the set it came from alone", () => {
  const set: ReadonlySet<string> = new Set(["a"]);
  assert.deepEqual([...toggled(set, "b")], ["a", "b"]);
  assert.deepEqual([...toggled(set, "a")], []);
  assert.deepEqual([...set], ["a"]);
});
