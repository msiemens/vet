import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import { arrowKey, closedKey, fileSymbolRows } from "./symbolRows.ts";

const PATH = "src/retry.rs";

function info(name: string[], line: number, path = PATH): SymbolInfo {
  const pos = { line, col: 0 };
  return {
    id: { path, name },
    kind: name.length > 1 ? "function" : "type",
    span: { start: pos, end: { line: line + 3, col: 0 } },
    at: pos,
  };
}

const changed = (name: string[], line: number, change: SymbolChange): ChangedSymbol => ({
  symbol: info(name, line),
  change,
});

const shape = (rows: ReturnType<typeof fileSymbolRows>) =>
  rows.map((r) => `${" ".repeat(r.depth)}${r.name}${r.change ? ` ${r.change}` : " (holder)"}`);

void test("symbols nest in the holders their names give, and a holder the change left alone is a muted row", () => {
  const rows = fileSymbolRows(
    PATH,
    [
      changed(["Policy", "next"], 10, "modified"),
      changed(["Policy", "jitter"], 20, "added"),
      changed(["Policy", "old"], 30, "removed"),
    ],
    [info(["Policy"], 5)],
    1,
    new Set(),
  );
  assert.deepEqual(shape(rows), [" Policy (holder)", "  next modified", "  jitter added", "  old removed"]);
  assert.deepEqual(
    rows.map((r) => r.holds),
    [true, false, false, false],
  );
});

void test("a changed holder is a symbol in its own right and holds the changed symbols in it", () => {
  const rows = fileSymbolRows(
    PATH,
    [changed(["mod tests"], 1, "added"), changed(["mod tests", "caps"], 3, "added")],
    [],
    1,
    new Set(),
  );
  assert.deepEqual(shape(rows), [" mod tests added", "  caps added"]);
});

void test("a symbol nests in the innermost holder its name gives", () => {
  const rows = fileSymbolRows(
    PATH,
    [changed(["Policy", "Backoff", "next"], 12, "modified")],
    [info(["Policy"], 5), info(["Policy", "Backoff"], 8)],
    1,
    new Set(),
  );
  assert.deepEqual(shape(rows), [" Policy (holder)", "  Backoff (holder)", "   next modified"]);
});

void test("what is in a closed holder is left out", () => {
  const all = [changed(["Policy", "next"], 10, "modified")];
  const context = [info(["Policy"], 5)];
  const holder = fileSymbolRows(PATH, all, context, 1, new Set())[0];
  const rows = fileSymbolRows(PATH, all, context, 1, new Set([closedKey(PATH, holder.key)]));
  assert.deepEqual(shape(rows), [" Policy (holder)"]);
});

void test("a symbol whose holder is not listed stands at the first level", () => {
  const rows = fileSymbolRows(PATH, [changed(["Missing", "inner"], 4, "modified")], [], 2, new Set());
  assert.deepEqual(shape(rows), ["  inner modified"]);
});

void test("two symbols with one name get distinct keys", () => {
  const rows = fileSymbolRows(PATH, [changed(["x"], 1, "modified"), changed(["x"], 9, "modified")], [], 1, new Set());
  assert.equal(new Set(rows.map((r) => r.key)).size, 2);
});

const tree = [
  { kind: "file", depth: 1, open: true, holds: true },
  { kind: "symbol", depth: 2, open: false, holds: true },
  { kind: "symbol", depth: 3, open: false, holds: false },
  { kind: "file", depth: 1, open: false, holds: true },
  { kind: "file", depth: 1, open: false, holds: false },
];
const holds = (r: (typeof tree)[number]) => r.holds;
const open = (r: (typeof tree)[number]) => r.open;

void test("→ opens a shut row, then moves into it", () => {
  assert.deepEqual(arrowKey(tree, 1, "right", holds, open), { toggle: tree[1] });
  assert.deepEqual(arrowKey(tree, 0, "right", holds, open), { move: 1 });
  assert.equal(arrowKey(tree, 2, "right", holds, open), undefined);
  assert.deepEqual(arrowKey(tree, 3, "right", holds, open), { toggle: tree[3] });
});

void test("← shuts an open row, else moves a symbol to the row that holds it", () => {
  assert.deepEqual(arrowKey(tree, 0, "left", holds, open), { toggle: tree[0] });
  assert.deepEqual(arrowKey(tree, 2, "left", holds, open), { move: 1 });
  assert.deepEqual(arrowKey(tree, 1, "left", holds, open), { move: 0 });
  assert.equal(arrowKey(tree, 3, "left", holds, open), undefined);
});

void test("← from a symbol skips its siblings to the row that holds it", () => {
  const rows = [
    { kind: "file", depth: 1, open: true, holds: true },
    { kind: "symbol", depth: 2, open: false, holds: false },
    { kind: "symbol", depth: 2, open: false, holds: false },
  ];
  assert.deepEqual(arrowKey(rows, 2, "left", holds, open), { move: 0 });
});
