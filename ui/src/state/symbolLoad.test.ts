import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { SymbolInfo } from "../generated/SymbolInfo";
import { analyseFiles, planLoad, reconcile, type Outcome } from "./symbolLoad.ts";

const backend = {};
const info = (path: string, name: string): SymbolInfo => ({
  id: { path, name: [name] },
  kind: "function",
  span: { start: { line: 1, col: 0 }, end: { line: 3, col: 0 } },
  at: { line: 1, col: 3 },
});

void test("a revision already loaded is not loaded again, unless the connection or servers asked for it", () => {
  const loaded = { backend, revision: 3 };
  assert.equal(planLoad(loaded, { backend, revision: 3 }, false), "skip");
  assert.equal(planLoad(loaded, { backend, revision: 3 }, true), "again");
  assert.equal(planLoad(loaded, { backend, revision: 4 }, false), "new");
  assert.equal(planLoad(loaded, { backend: {}, revision: 3 }, false), "new");
  assert.equal(planLoad(undefined, { backend, revision: 3 }, false), "new");
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

/** Asks that the test answers one by one. */
function pending() {
  const asked: { path: string; signal: AbortSignal; answer: (v: string) => void; fail: (e: unknown) => void }[] = [];
  const done: [string, Outcome<string>][] = [];
  const ask = (path: string, signal: AbortSignal) =>
    new Promise<string>((answer, fail) => asked.push({ path, signal, answer, fail }));
  return { asked, done, ask };
}
const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

void test("files are asked a few at a time, the file on view first", async () => {
  const { asked, done, ask } = pending();
  let current: string | undefined = "d.rs";
  analyseFiles({
    paths: ["a.rs", "b.rs", "c.rs", "d.rs"],
    current: () => current,
    atOnce: 2,
    ask,
    done: (path, outcome) => done.push([path, outcome]),
    signal: new AbortController().signal,
  });
  assert.deepEqual(
    asked.map((a) => a.path),
    ["d.rs", "a.rs"],
  );
  current = "c.rs";
  asked[0].answer("d");
  await settle();
  assert.deepEqual(
    asked.map((a) => a.path),
    ["d.rs", "a.rs", "c.rs"],
  );
  assert.deepEqual(done, [["d.rs", { ok: true, value: "d" }]]);
});

void test("an analysis that is superseded reports nothing more and asks for no more files", async () => {
  const { asked, done, ask } = pending();
  const stop = new AbortController();
  analyseFiles({
    paths: ["a.rs", "b.rs", "c.rs"],
    current: () => undefined,
    atOnce: 1,
    ask,
    done: (path, outcome) => done.push([path, outcome]),
    signal: stop.signal,
  });
  stop.abort();
  asked[0].answer("late");
  await settle();
  assert.deepEqual(done, []);
  assert.equal(asked.length, 1);
});

void test("a failed file is reported and the others go on; a cancelled one is no outcome", async () => {
  const { asked, done, ask } = pending();
  analyseFiles({
    paths: ["a.rs", "b.rs", "c.rs"],
    current: () => undefined,
    atOnce: 1,
    ask,
    done: (path, outcome) => done.push([path, outcome]),
    signal: new AbortController().signal,
  });
  const boom = new Error("boom");
  asked[0].fail(boom);
  await settle();
  asked[1].answer("b");
  await settle();
  assert.deepEqual(done, [
    ["a.rs", { ok: false, error: boom }],
    ["b.rs", { ok: true, value: "b" }],
  ]);
  asked[2].fail(new DOMException("aborted", "AbortError"));
  await settle();
  assert.equal(done.length, 2);
});
