import assert from "node:assert/strict";
import { test } from "node:test";
import type { LanguageSymbols } from "../../generated/LanguageSymbols";
import { abortError } from "../../lib/abort.ts";
import { type Found, SymbolSearch } from "./symbolSearch.ts";

/** A search whose answers are given by hand, in any order; a server may fail a search that was cancelled, or answer it anyway. */
function harness(rejectsCancelled = false) {
  const asked: { query: string; signal: AbortSignal; answer: (languages: LanguageSymbols[]) => void }[] = [];
  const applied: Found[] = [];
  const failed: string[] = [];
  const busy: boolean[] = [];
  const search = new SymbolSearch(
    (query, _revision, signal) =>
      new Promise<LanguageSymbols[]>((resolve, reject) => {
        if (rejectsCancelled) signal.addEventListener("abort", () => reject(abortError()));
        asked.push({ query, signal, answer: resolve });
      }),
    {
      busy: (on) => busy.push(on),
      apply: (found) => applied.push(found),
      fail: (query) => failed.push(query),
    },
  );
  return { search, asked, applied, failed, busy };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const named = (language: string): LanguageSymbols[] => [{ language, symbols: [], failure: null }];

void test("an answer that comes after a newer search began is dropped", async () => {
  const { search, asked, applied } = harness();
  search.search("re", 1);
  search.search("retry", 1);
  // The older search is cancelled, but a server may answer it anyway.
  asked[0].answer(named("old"));
  asked[1].answer(named("new"));
  await tick();
  assert.deepEqual(
    applied.map((f) => [f.query, f.languages[0].language]),
    [["retry", "new"]],
  );
});

void test("a search is cancelled down to the request when the next one begins or it is cancelled", async () => {
  const { search, asked, applied, busy } = harness();
  search.search("a", 1);
  search.search("ab", 1);
  assert.equal(asked[0].signal.aborted, true);
  assert.equal(asked[1].signal.aborted, false);
  search.cancel();
  assert.equal(asked[1].signal.aborted, true);
  asked[1].answer(named("late"));
  await tick();
  assert.deepEqual(applied, []);
  assert.equal(busy.at(-1), false);
});

void test("a failure of the latest search is told, a cancelled search's is not", async () => {
  const { search, failed } = harness(true);
  search.search("a", 1);
  search.search("ab", 1);
  await tick();
  assert.deepEqual(failed, []);
  const rejecting = new SymbolSearch(() => Promise.reject(new Error("boom")), {
    apply: () => undefined,
    fail: (query) => failed.push(query),
  });
  rejecting.search("x", 2);
  await tick();
  assert.deepEqual(failed, ["x"]);
});
