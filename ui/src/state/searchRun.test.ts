import assert from "node:assert/strict";
import { test } from "node:test";
import { abortError } from "../lib/abort.ts";
import type { SearchQuery, SearchResults } from "./backend.ts";
import { SearchRun } from "./searchRun.ts";

const query = (text: string) => ({ pattern: { text } }) as SearchQuery;
const results = (hits: number) => ({ hits }) as SearchResults;

/** A backend whose searches answer when told and reject like a cancelled request when aborted. */
function fake() {
  const sent: { query: SearchQuery; signal?: AbortSignal; answer: (r: SearchResults) => void }[] = [];
  const backend = {
    search(q: SearchQuery, signal?: AbortSignal) {
      return new Promise<SearchResults>((resolve, reject) => {
        sent.push({ query: q, signal, answer: resolve });
        signal?.addEventListener("abort", () => reject(abortError()));
      });
    },
  };
  return { backend, sent };
}

const later = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

void test("asks within the pause send one request, with the last query", async () => {
  const { backend, sent } = fake();
  const run = new SearchRun();
  const first = run.ask(backend, query("a"), 50);
  const second = run.ask(backend, query("ab"), 50);
  const third = run.ask(backend, query("abc"), 50);
  await later(100);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].query.pattern.text, "abc");
  sent[0].answer(results(3));
  assert.equal(await first, undefined);
  assert.equal(await second, undefined);
  assert.deepEqual(await third, { type: "done", value: results(3) });
});

void test("an older answer that comes while the next ask waits out its pause is dropped", async () => {
  const { backend, sent } = fake();
  const run = new SearchRun();
  const a = run.ask(backend, query("a"));
  const b = run.ask(backend, query("b"), 50);
  assert.equal(sent[0].signal?.aborted, true);
  sent[0].answer(results(1));
  assert.equal(await a, undefined);
  await later(100);
  assert.equal(sent.length, 2);
  sent[1].answer(results(2));
  assert.deepEqual(await b, { type: "done", value: results(2) });
});

void test("cancel during the pause sends nothing", async () => {
  const { backend, sent } = fake();
  const run = new SearchRun();
  const asked = run.ask(backend, query("a"), 50);
  run.cancel();
  assert.equal(await asked, undefined);
  await later(100);
  assert.equal(sent.length, 0);
});
