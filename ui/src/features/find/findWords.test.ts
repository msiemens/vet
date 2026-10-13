import assert from "node:assert/strict";
import { test } from "node:test";
import { countText, cutText, rerunText, staleText, tabTitle } from "./findWords.ts";

void test("the stale banner names the changed files by name, or says only that the code changed", () => {
  assert.equal(
    staleText(["src/sync/job.rs", "src/http/client.rs"]),
    "The code changed since this ran (job.rs, client.rs). Rows in changed files may have moved.",
  );
  assert.equal(staleText([]), "The code changed since this ran. Rows in changed files may have moved.");
});

void test("the counts read in the singular for one and with thousands separated", () => {
  assert.equal(countText(1, 1, 0), "1 result in 1 file");
  assert.equal(countText(7, 6, 2), "7 results in 6 files · 2 not seen");
  assert.equal(cutText(200, 1340), "Showing the first 200 of 1,340.");
  assert.equal(rerunText({ fresh: 1, gone: 0 }), "Rerun: 1 new, 0 gone.");
});

void test("a tab says whether the next result replaces it", () => {
  assert.equal(tabTitle("usages", "next_delay", false), "Usages of next_delay · replaced by the next result");
  assert.equal(tabTitle("search", "jitter", true), "Find in Files: jitter · pinned");
});
