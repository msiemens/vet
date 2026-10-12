import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../../generated/ChangedSymbol.ts";
import type { SymbolChange } from "../../generated/SymbolChange.ts";
import { describeFate, fileEntries, groupByFile, rowKeys, rowLabel, searchEntries } from "./symbolList.ts";

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

void test("a row names the symbol and the innermost container, not the namespace chain", () => {
  const label = (path: string, name: string[]) => rowLabel({ path, name }, path);
  assert.deepEqual(label("src/Invoice.ts", ["Acme.Billing.Core", "Invoice", "merge"]), {
    name: "merge",
    context: "Invoice",
  });
  assert.deepEqual(label("billing/core.rs", ["billing", "core", "impl Invoice", "merge"]), {
    name: "merge",
    context: "impl Invoice",
  });
  assert.deepEqual(label("billing/invoice.py", ["Invoice", "merge"]), { name: "merge", context: "Invoice" });
});

void test("a symbol at the top level of its file is placed in the file", () => {
  assert.deepEqual(rowLabel({ path: "src/billing/util.py", name: ["total"] }, "src/billing/util.py"), {
    name: "total",
    context: "util.py",
  });
});

void test("a place without a symbol is its file, whatever the separators of its path", () => {
  assert.deepEqual(rowLabel(null, "src/billing/util.py"), { name: "util.py", context: "" });
  assert.deepEqual(rowLabel(null, "/home/dep/lib/mod.rs"), { name: "mod.rs", context: "" });
  assert.deepEqual(rowLabel(null, "C:\\dep\\Mod.cs"), { name: "Mod.cs", context: "" });
});

void test("a C# method is named without its parameters and return type", () => {
  const id = {
    path: "src/Invoice.cs",
    name: ["Acme.Billing.Core", "Invoice", "Merge(Dictionary<string, List<Line>>, Task<Line>) : List<Line>"],
  };
  assert.deepEqual(rowLabel(id, id.path), { name: "Merge", context: "Invoice" });
  // The same goes for a method that contains the place, and for a constructor.
  const local = { path: id.path, name: ["Acme", "Invoice", "Merge(int x) : void", "Helper(int y) : int"] };
  assert.deepEqual(rowLabel(local, local.path), { name: "Helper", context: "Merge" });
  const constructor = { path: id.path, name: ["Acme", "Invoice", "Invoice(int id)"] };
  assert.deepEqual(rowLabel(constructor, constructor.path), { name: "Invoice", context: "Invoice" });
});

void test("parentheses in names of other languages are kept", () => {
  const id = { path: "src/a.ts", name: ["Invoice", "(anonymous)"] };
  assert.equal(rowLabel(id, id.path).name, "(anonymous)");
  const callback = { path: "src/a.ts", name: ["run", "map() callback"] };
  assert.equal(rowLabel(callback, callback.path).name, "map() callback");
});

void test("rows of symbols with the same id get distinct keys, and the same rows the same keys again", () => {
  // Two object literals with a property `a` in one test file.
  const [entry] = groupByFile([
    sym("t.ts", ["a"], 3, 3, "added"),
    sym("t.ts", ["a"], 9, 9, "added"),
    sym("t.ts", ["a"], 12, 12, "removed"),
    sym("t.ts", ["b"], 14, 14, "added"),
  ]);
  const keys = rowKeys(entry.symbols);
  assert.equal(new Set(keys).size, keys.length);
  assert.deepEqual(rowKeys(entry.symbols), keys);
});
