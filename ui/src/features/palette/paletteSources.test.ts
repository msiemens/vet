import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { LanguageSymbols } from "../../generated/LanguageSymbols";
import type { ServerInfo } from "../../generated/ServerInfo";
import type { Task } from "../../generated/Task";
import type { Thread } from "../../generated/Thread";
import type { Unavailable } from "../../generated/Unavailable";
import type { UnavailableKind } from "../../generated/UnavailableKind";
import {
  failedServers,
  indexingNotes,
  serverSymbolCandidates,
  symbolCandidates,
  threadCandidates,
} from "./paletteSources.ts";

const symbol = (path: string, name: string[], change: ChangedSymbol["change"] = "modified"): ChangedSymbol => ({
  change,
  symbol: {
    id: { path, name },
    modules: 0,
    kind: "function",
    span: { start: { line: 39, col: 0 }, end: { line: 50, col: 1 } },
    at: { line: 39, col: 7 },
  },
});

void test("a member symbol is named after its container in its language's way, a top-level one by its name", () => {
  const found = symbolCandidates([
    {
      symbols: [symbol("src/retry.rs", ["RetryPolicy", "next_delay"]), symbol("src/retry.rs", ["should_retry"])],
      seen: 20,
    },
    { symbols: [symbol("src/retry.ts", ["RetryPolicy", "nextDelay"])], seen: 100 },
  ]);
  assert.deepEqual(
    found.map((s) => [s.label, s.lead]),
    [
      ["RetryPolicy::next_delay", 13],
      ["should_retry", 0],
      ["RetryPolicy.nextDelay", 12],
    ],
  );
  assert.equal(found[0].seen, 20);
  assert.equal(found[0].change, "modified");
});

void test("two symbols of one name that changed differently are two candidates", () => {
  const found = symbolCandidates([
    { symbols: [symbol("a.rs", ["f"], "removed"), symbol("a.rs", ["f"], "added")], seen: 0 },
  ]);
  assert.equal(new Set(found.map((s) => s.key)).size, 2);
});

const failure = (kind: UnavailableKind, message = "why"): Unavailable => ({ kind, label: "", message, language: null });
const answer = (language: string, symbols: ChangedSymbol[], failed: Unavailable | null = null): LanguageSymbols => ({
  language,
  symbols: symbols.map((s) => s.symbol),
  failure: failed,
});
const at = (changed: ChangedSymbol, line: number): ChangedSymbol => ({
  ...changed,
  symbol: { ...changed.symbol, at: { line, col: 7 } },
});
const nothing: ReadonlySet<string> = new Set();

void test("a symbol both the changeset and a server hold is listed once, as the changed one", () => {
  const changed = symbolCandidates([{ symbols: [symbol("a.rs", ["f"], "added")], seen: 0 }]);
  const found = serverSymbolCandidates(
    [answer("rust", [symbol("a.rs", ["f"]), symbol("b.rs", ["g"]), symbol("b.rs", ["g"])])],
    changed,
    nothing,
  );
  assert.deepEqual(
    found.map((s) => [s.info.id.path, s.label, s.change, s.fromServer]),
    [["b.rs", "g", undefined, true]],
  );
});

void test("an overload of a changed symbol is another symbol, and a removed symbol hides none", () => {
  const changed = symbolCandidates([
    { symbols: [symbol("A.cs", ["Run"], "modified"), at(symbol("A.cs", ["Gone"], "removed"), 70)], seen: 0 },
  ]);
  const found = serverSymbolCandidates(
    [answer("csharp", [symbol("A.cs", ["Run"]), at(symbol("A.cs", ["Run"]), 80), at(symbol("A.cs", ["Gone"]), 70)])],
    changed,
    nothing,
  );
  assert.deepEqual(
    found.map((s) => [s.label, s.info.at.line]),
    [
      ["Run", 80],
      ["Gone", 70],
    ],
  );
});

void test("the servers' symbols of a file whose changed symbols are not known are left out", () => {
  const found = serverSymbolCandidates(
    [answer("rust", [symbol("a.rs", ["f"]), symbol("b.rs", ["g"])])],
    [],
    new Set(["a.rs"]),
  );
  assert.deepEqual(
    found.map((s) => s.info.id.path),
    ["b.rs"],
  );
});

void test("a language whose server failed is named by its server, with why and what can be done", () => {
  const servers = [
    { language: "python", server: "Pyright" },
    { language: "go", server: "gopls" },
    { language: "csharp", server: "Roslyn" },
    { language: "rust", server: "rust-analyzer" },
  ] as ServerInfo[];
  const found = failedServers(
    [
      answer("rust", []),
      answer("python", [], failure("serverMissing", "not installed")),
      answer("go", [], failure("analysisLimited")),
      answer("csharp", [], failure("serverFailed")),
    ],
    servers,
  );
  assert.deepEqual(
    found.map((f) => [f.server, f.message, f.actions]),
    [
      ["Pyright", "not installed", ["settings"]],
      ["gopls", "why", []],
      ["Roslyn", "why", ["restart", "settings"]],
    ],
  );
});

const server = { language: "rust", server: "rust-analyzer" } as ServerInfo;
const indexing = (title: string, done?: number): Task =>
  ({ kind: "serverIndex", title, done: done ?? null, total: done === undefined ? null : 100 }) as Task;

void test("a server still indexing is said to make the symbols incomplete, a failed or idle one is not", () => {
  const ok = [answer("rust", [])];
  assert.deepEqual(indexingNotes(ok, [server], [indexing("Indexing rust-analyzer", 62)]), [
    "rust-analyzer is still indexing (62%). Symbols of unchanged files may be incomplete.",
  ]);
  assert.deepEqual(indexingNotes(ok, [server], [indexing("Indexing Pyright")]), []);
  assert.deepEqual(
    indexingNotes([answer("rust", [], failure("serverFailed"))], [server], [indexing("Indexing rust-analyzer")]),
    [],
  );
});

const thread = (over: Partial<Thread>): Thread =>
  ({
    id: 1,
    kind: "range",
    side: "target",
    path: "src/retry.rs",
    original: null,
    location: { path: "src/retry.rs", start: 42, len: 1 },
    targetLine: 42,
    anchor: { type: "exact", whitespaceChanged: false },
    messages: [{ body: "Jitter is added\nafter the cap" }],
    status: "open",
    platform: null,
    ...over,
  }) as Thread;

void test("a thread is found by its first message, shown at its place with its state", () => {
  const [open, moved, resolved, whole] = threadCandidates([
    thread({}),
    thread({ id: 2, anchor: { type: "inferred", strategy: "context", confidence: "high" } as Thread["anchor"] }),
    thread({ id: 3, status: "resolved" }),
    thread({ id: 4, kind: "changeset", path: null, location: null, anchor: null }),
  ]);
  assert.equal(open.text, "Jitter is added after the cap");
  assert.equal(open.where, "src/retry.rs:43");
  assert.deepEqual([open.moved, moved.moved, resolved.resolved, whole.where], [false, true, true, "changeset"]);
});
