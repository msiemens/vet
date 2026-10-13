import assert from "node:assert/strict";
import { test } from "node:test";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import { type ChoiceHost, openChoice } from "./openChoice.ts";

const symbol = { id: { path: "src/retry.rs", name: ["f"] }, span: { start: { line: 41, col: 0 } } } as SymbolInfo;

/** A review screen that notes what is done to it, with a commit on screen if asked. */
function host(commitShown: boolean) {
  const did: string[] = [];
  let shown = commitShown;
  const screen: ChoiceHost = {
    showReview: () => did.push("review"),
    commitShown: () => shown,
    showWhole: async () => {
      await Promise.resolve();
      shown = false;
      did.push("whole");
    },
    openOnScreen: (path, line) => did.push(`on screen ${path}:${line}`),
    openUnpinned: (path, line) => did.push(`unpinned ${path}:${line}`),
    revealFile: (path) => did.push(`reveal ${path}`),
    revealSymbol: (s, change) => did.push(`reveal symbol ${s.id.path} ${change}${shown ? " in a commit" : ""}`),
    openChangeset: (id) => did.push(`changeset ${id}`),
    openThread: (id) => did.push(`thread ${id}`),
    run: (command) => did.push(`run ${command}`),
  };
  return { did, screen };
}

void test("a removed symbol is shown among the changes of the whole changeset, not of the commit on screen", async () => {
  const { did, screen } = host(true);
  await openChoice({ type: "symbol", symbol, change: "removed", reveal: false }, screen);
  assert.deepEqual(did, ["review", "whole", "reveal symbol src/retry.rs removed"]);
});

void test("a changed symbol revealed on request, and a file shown in the changes, do the same", async () => {
  const symbolRun = host(true);
  await openChoice({ type: "symbol", symbol, change: "modified", reveal: true }, symbolRun.screen);
  assert.deepEqual(symbolRun.did, ["review", "whole", "reveal symbol src/retry.rs modified"]);

  const fileRun = host(true);
  await openChoice({ type: "file", path: "a.rs", inChanges: true }, fileRun.screen);
  assert.deepEqual(fileRun.did, ["review", "whole", "reveal a.rs"]);

  const wholeRun = host(false);
  await openChoice({ type: "file", path: "a.rs", inChanges: true }, wholeRun.screen);
  assert.deepEqual(wholeRun.did, ["review", "reveal a.rs"]);
});

void test("a symbol that is opened is opened in the changeset's own tab, with a commit on screen or not", async () => {
  for (const commit of [true, false]) {
    const { did, screen } = host(commit);
    await openChoice({ type: "symbol", symbol, reveal: false }, screen);
    assert.deepEqual(did, ["review", "unpinned src/retry.rs:41"]);
  }
});

void test("a file opens as the view on screen has it, at the line asked for", async () => {
  const { did, screen } = host(true);
  await openChoice({ type: "file", path: "a.rs", line: 9, inChanges: false }, screen);
  assert.deepEqual(did, ["review", "on screen a.rs:9"]);
});
