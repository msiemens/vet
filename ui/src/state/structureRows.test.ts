import assert from "node:assert/strict";
import { test } from "node:test";
import type { OutlineSymbol } from "../generated/OutlineSymbol";
import type { SymbolChange } from "../generated/SymbolChange";
import { keyOf } from "./symbolKey.ts";
import { enclosingId, holderKeys, holdersOf, structureRows, type RowOptions } from "./structureRows.ts";

const PATH = "src/retry.rs";

function sym(name: string[], from: number, to: number, change?: SymbolChange, col = 0): OutlineSymbol {
  return {
    symbol: {
      id: { path: PATH, name },
      modules: 0,
      kind: name.length > 1 ? "function" : "type",
      span: { start: { line: from, col }, end: { line: to, col: 0 } },
      at: { line: from, col },
      test: false,
    },
    change: change ?? null,
  };
}

// Policy { zeta, alpha (modified) }, helper (modified), Gone (removed) in the order a reading gives.
const OUTLINE = [
  sym(["Policy"], 0, 10),
  sym(["Policy", "zeta"], 1, 3),
  sym(["Policy", "alpha"], 4, 8, "modified"),
  sym(["helper"], 12, 14, "modified"),
  sym(["Gone"], 20, 22, "removed"),
  sym(["tail"], 30, 32),
];

const OPEN: RowOptions = { sort: false, changedOnly: false, query: "", closed: new Set() };
const shape = (rows: ReturnType<typeof structureRows>) =>
  rows.map((r) => `${" ".repeat(r.depth)}${r.name}${r.change ? ` ${r.change}` : ""}${r.muted ? " (muted)" : ""}`);

void test("the symbols of a file nest in the holders their names give, in the order they were listed", () => {
  assert.deepEqual(shape(structureRows(OUTLINE, OPEN)), [
    "Policy",
    " zeta",
    " alpha modified",
    "helper modified",
    "Gone removed",
    "tail",
  ]);
});

void test("sorting orders the symbols under each holder alphabetically and leaves the holders where they are nested", () => {
  assert.deepEqual(shape(structureRows(OUTLINE, { ...OPEN, sort: true })), [
    "Gone removed",
    "helper modified",
    "Policy",
    " alpha modified",
    " zeta",
    "tail",
  ]);
});

void test("changed only lists the changed symbols and the holders that place them, muted", () => {
  const rows = structureRows(OUTLINE, { ...OPEN, changedOnly: true });
  assert.deepEqual(shape(rows), ["Policy (muted)", " alpha modified", "helper modified", "Gone removed"]);
  assert.deepEqual(
    rows.map((r) => r.holds),
    [true, false, false, false],
  );
});

void test("a search keeps the matches and their holders, opens shut holders and says what matched", () => {
  const closed = new Set([keyOf(OUTLINE[0].symbol.id)]);
  const rows = structureRows(OUTLINE, { ...OPEN, query: "alp", closed });
  assert.deepEqual(shape(rows), ["Policy (muted)", " alpha modified"]);
  assert.deepEqual(rows[1].ranges, [[0, 3]]);
  assert.deepEqual(structureRows(OUTLINE, { ...OPEN, query: "nothing" }), []);
});

void test("the contents of a shut holder are left out", () => {
  const closed = new Set([keyOf(OUTLINE[0].symbol.id)]);
  const rows = structureRows(OUTLINE, { ...OPEN, closed });
  assert.deepEqual(shape(rows), ["Policy", "helper modified", "Gone removed", "tail"]);
  assert.equal(rows[0].open, false);
});

void test("two symbols with one name are two rows with keys of their own", () => {
  const rows = structureRows([sym(["impl Policy"], 0, 3), sym(["impl Policy"], 4, 8)], OPEN);
  assert.equal(new Set(rows.map((r) => r.key)).size, 2);
});

void test("the holders are the symbols others nest in, and a symbol knows the holders around it", () => {
  assert.deepEqual(holderKeys(OUTLINE), [keyOf(OUTLINE[0].symbol.id)]);
  assert.deepEqual(holdersOf(OUTLINE, keyOf(OUTLINE[2].symbol.id)), [keyOf(OUTLINE[0].symbol.id)]);
  assert.deepEqual(holdersOf(OUTLINE, keyOf(OUTLINE[5].symbol.id)), []);
});

void test("the cursor is in the innermost symbol that holds it, never in a removed one", () => {
  const at = (line: number, col = 0) => enclosingId(OUTLINE, { line, col });
  assert.equal(at(5), keyOf(OUTLINE[2].symbol.id));
  assert.equal(at(9), keyOf(OUTLINE[0].symbol.id), "between the members, in the holder");
  assert.equal(at(1), keyOf(OUTLINE[1].symbol.id));
  assert.equal(at(11), undefined, "outside every symbol");
  assert.equal(at(21), undefined, "a removed symbol has no place in the target");
});

void test("of two symbols that start together the smaller one holds the cursor", () => {
  const outline = [sym(["impl P"], 0, 0, undefined, 0), sym(["impl P", "f"], 0, 0, undefined, 0)];
  outline[0].symbol.span.end = { line: 0, col: 30 };
  outline[1].symbol.span.end = { line: 0, col: 20 };
  assert.equal(enclosingId(outline, { line: 0, col: 10 }), keyOf(outline[1].symbol.id));
  assert.equal(enclosingId(outline, { line: 0, col: 25 }), keyOf(outline[0].symbol.id));
});
