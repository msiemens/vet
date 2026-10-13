import assert from "node:assert/strict";
import { test } from "node:test";
import { describeFate, placeShort } from "./symbolDetail.ts";

void test("fates read as words, the broken one as the bad tone", () => {
  assert.equal(describeFate({ type: "likelyBroken" }).tone, "bad");
  const moved = describeFate({
    type: "bindingChanged",
    now: [{ path: "u.rs", span: { start: { line: 9, col: 0 }, end: { line: 9, col: 3 } }, external: false }],
  });
  assert.deepEqual([moved.label, moved.detail], ["binds elsewhere now", "u.rs:10"]);
  assert.equal(describeFate({ type: "unknown", reason: "no server" }).detail, "no server");
});

void test("a place with little room names the file and the 1-based line", () => {
  assert.equal(placeShort("src/sync/job.rs", { line: 117, col: 4 }), "job.rs:118");
  assert.equal(placeShort("C:\\repo\\job.rs", { line: 0, col: 0 }), "job.rs:1");
});
