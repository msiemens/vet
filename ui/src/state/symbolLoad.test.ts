import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { SymbolInfo } from "../generated/SymbolInfo";
import { reconcile } from "./symbolLoad.ts";

const info = (path: string, name: string): SymbolInfo => ({
  id: { path, name: [name] },
  kind: "function",
  span: { start: { line: 1, col: 0 }, end: { line: 3, col: 0 } },
  at: { line: 1, col: 3 },
});

void test("a symbol on show is updated to what the new analysis says, and unchanged if it left the changed ones", () => {
  const shown = { symbol: info("a.rs", "f"), change: "modified" as const };
  const now = info("a.rs", "f");
  now.at.line = 7;
  const added: ChangedSymbol = { symbol: now, change: "added" };
  assert.deepEqual(reconcile(shown, { path: "a.rs", symbols: [added] }), { symbol: now, change: "added" });
  assert.deepEqual(reconcile(shown, { path: "a.rs", symbols: [] }), { symbol: shown.symbol, change: undefined });
  // Another file's analysis says nothing about it.
  assert.equal(reconcile(shown, { path: "b.rs", symbols: [] }), shown);
});
