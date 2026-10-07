import assert from "node:assert/strict";
import { test } from "node:test";
import type { BaseDiff } from "../../../generated/BaseDiff.ts";
import type { CommitInfo } from "../../../generated/CommitInfo.ts";
import { baseDiffTitle } from "./baseDiffTitle.ts";

const commit = (shortId: string) => ({ shortId }) as CommitInfo;

void test("the diff names the base the hunk was marked against and the base now", () => {
  const diff: BaseDiff = { from: commit("a41c9e2"), to: commit("c03d55f"), rows: [] };
  assert.equal(baseDiffTitle(diff), "base a41c9e2 → c03d55f");
});

void test("a base git has dropped is left unnamed", () => {
  assert.equal(baseDiffTitle({ from: null, to: commit("c03d55f"), rows: [] }), "base → c03d55f");
});
