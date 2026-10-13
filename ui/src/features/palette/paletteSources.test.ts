import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { LanguageSymbols } from "../../generated/LanguageSymbols";
import type { ServerInfo } from "../../generated/ServerInfo";
import type { Task } from "../../generated/Task";
import type { Thread } from "../../generated/Thread";
import {
  actionCandidates,
  failedServers,
  indexingNotes,
  serverSymbolCandidates,
  symbolCandidates,
  threadCandidates,
  unchangedCandidates,
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

void test("the unchanged files are those of the project the changeset does not change", () => {
  const found = unchangedCandidates(["a.rs", "b.rs", "c.rs"], new Set(["b.rs"]));
  assert.deepEqual(
    found.map((f) => [f.path, f.changed]),
    [
      ["a.rs", false],
      ["c.rs", false],
    ],
  );
});

void test("two symbols of one name that changed differently are two candidates", () => {
  const found = symbolCandidates([
    { symbols: [symbol("a.rs", ["f"], "removed"), symbol("a.rs", ["f"], "added")], seen: 0 },
  ]);
  assert.equal(new Set(found.map((s) => s.key)).size, 2);
});

const answer = (language: string, symbols: ChangedSymbol[], failure: string | null = null): LanguageSymbols => ({
  language,
  symbols: symbols.map((s) => s.symbol),
  failure,
});

void test("a symbol both the changeset and a server hold is listed once, as the changed one", () => {
  const changed = symbolCandidates([{ symbols: [symbol("a.rs", ["f"], "added")], seen: 0 }]);
  const found = serverSymbolCandidates(
    [answer("rust", [symbol("a.rs", ["f"]), symbol("b.rs", ["g"]), symbol("b.rs", ["g"])])],
    changed,
  );
  assert.deepEqual(
    found.map((s) => [s.info.id.path, s.label, s.change, s.fromServer]),
    [["b.rs", "g", undefined, true]],
  );
});

void test("a language whose server failed is named by its server, with why", () => {
  const servers = [{ language: "python", server: "Pyright" }] as ServerInfo[];
  const found = failedServers([answer("rust", []), answer("python", [], "not installed")], servers);
  assert.deepEqual(found, [{ language: "python", server: "Pyright", failure: "not installed" }]);
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
  assert.deepEqual(indexingNotes([answer("rust", [], "boom")], [server], [indexing("Indexing rust-analyzer")]), []);
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

void test("the popup's own commands are not among the actions", () => {
  const found = actionCandidates(
    [
      { id: "palette.files", label: "Go to File…", enabled: true },
      { id: "review.post", label: "Post Comments…", enabled: false, reason: "No comment waits" },
    ],
    (id) => id.startsWith("palette."),
  );
  assert.deepEqual(
    found.map((a) => [a.key, a.off, a.disabled]),
    [["review.post", true, "No comment waits"]],
  );
});
