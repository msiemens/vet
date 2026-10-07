import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../../generated/ChangedSymbol.ts";
import type { SymbolChange } from "../../generated/SymbolChange.ts";
import { describeFate, fileEntries, groupByFile, symbolAt } from "./symbolList.ts";

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
    unsupported: ["notes.txt", "a.rs"],
    gaps: [{ path: "b.rs", reason: "parse failed" }],
    inputs: { analysis: "changed_symbols", bounds: null, primitives: [], hunks: [], config: [] },
  });
  assert.deepEqual(
    entries.map((e) => [e.path, e.symbols.length, e.note]),
    [
      ["a.rs", 1, undefined],
      ["b.rs", 0, "parse failed"],
      ["notes.txt", 0, "Language unsupported"],
    ],
  );
});

void test("the innermost symbol at a position wins", () => {
  const all = [sym("a.rs", ["Outer"], 0, 20, "modified"), sym("a.rs", ["Outer", "inner"], 5, 8, "added")];
  assert.deepEqual(symbolAt(all, "a.rs", "target", 6, 2)?.symbol.id.name, ["Outer", "inner"]);
  assert.deepEqual(symbolAt(all, "a.rs", "target", 10, 0)?.symbol.id.name, ["Outer"]);
  // the end is exclusive
  assert.equal(symbolAt(all, "a.rs", "target", 20, 1), undefined);
  assert.equal(symbolAt(all, "other.rs", "target", 6, 2), undefined);
});

void test("a removed symbol is found on the base side only", () => {
  const all = [sym("a.rs", ["gone"], 3, 6, "removed"), sym("a.rs", ["kept"], 3, 6, "modified")];
  assert.deepEqual(symbolAt(all, "a.rs", "base", 4, 0)?.symbol.id.name, ["gone"]);
  assert.deepEqual(symbolAt(all, "a.rs", "target", 4, 0)?.symbol.id.name, ["kept"]);
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
