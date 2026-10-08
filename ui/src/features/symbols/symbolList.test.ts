import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../../generated/ChangedSymbol.ts";
import type { SymbolChange } from "../../generated/SymbolChange.ts";
import { describeFate, fileEntries, groupByFile, rowClick, searchEntries } from "./symbolList.ts";

const sym = (path: string, name: string[], from: number, to: number, change: SymbolChange): ChangedSymbol => ({
  symbol: {
    id: { path, name },
    span: { start: { line: from, col: 0 }, end: { line: to, col: 1 } },
    at: { line: from, col: 4 },
  },
  change,
});

void test("symbols group by file, files and symbols in order", () => {
  const groups = groupByFile([
    sym("b.rs", ["late"], 20, 22, "added"),
    sym("a.rs", ["x"], 5, 6, "modified"),
    sym("b.rs", ["early"], 1, 3, "removed"),
  ]);
  assert.deepEqual(
    groups.map((g) => [g.path, g.symbols.map((s) => s.symbol.id.name[0])]),
    [
      ["a.rs", ["x"]],
      ["b.rs", ["early", "late"]],
    ],
  );
});

void test("files without symbols are listed with the reason", () => {
  const entries = fileEntries({
    revision: 1,
    symbols: [sym("a.rs", ["x"], 1, 2, "added")],
    gaps: [
      { path: "c.rs", reason: "Language server failed" },
      { path: "a.rs", reason: "parse failed" },
      { path: "b.rs", reason: "parse failed" },
    ],
    inputs: { analysis: "changed_symbols", bounds: null, primitives: [], hunks: [], config: [] },
  });
  assert.deepEqual(
    entries.map((e) => [e.path, e.symbols.length, e.note]),
    [
      ["a.rs", 1, undefined],
      ["b.rs", 0, "parse failed"],
      ["c.rs", 0, "Language server failed"],
    ],
  );
});

void test("a search keeps the matching symbols under their files and drops files without one", () => {
  const entries = groupByFile([
    sym("a.rs", ["Parser", "parseHeader"], 1, 3, "added"),
    sym("a.rs", ["Parser", "next"], 4, 6, "modified"),
    sym("b.rs", ["Other"], 1, 2, "removed"),
  ]);
  const { entries: found, ranges } = searchEntries(entries, "pH");
  assert.deepEqual(
    found.map((e) => [e.path, e.symbols.map((s) => s.symbol.id.name.at(-1))]),
    [["a.rs", ["parseHeader"]]],
  );
  assert.deepEqual(ranges.get(found[0].symbols[0]), [
    [0, 1],
    [5, 6],
  ]);
});

void test("fates read as words, the broken one as the bad tone", () => {
  assert.equal(describeFate({ type: "likelyBroken" }).tone, "bad");
  const moved = describeFate({
    type: "bindingChanged",
    now: [{ path: "u.rs", span: { start: { line: 9, col: 0 }, end: { line: 9, col: 3 } }, external: false }],
  });
  assert.deepEqual([moved.label, moved.detail], ["binds elsewhere now", "u.rs:10"]);
  assert.equal(describeFate({ type: "unknown", reason: "no server" }).detail, "no server");
});

void test("a double click selects on its first click and reveals on its second without selecting again", () => {
  assert.deepEqual(rowClick(1, false), { select: true, reveal: false });
  assert.deepEqual(rowClick(2, true), { select: false, reveal: true });
});

void test("a double click whose first click went to another row selects as it reveals", () => {
  assert.deepEqual(rowClick(2, false), { select: true, reveal: true });
});

void test("a click from the keyboard selects", () => {
  assert.deepEqual(rowClick(0, false), { select: true, reveal: false });
});
