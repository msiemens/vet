import assert from "node:assert/strict";
import { test } from "node:test";
import type { Completion } from "../../generated/Completion.ts";
import { completionTitle } from "./completionTitle.ts";

const completion = (c: Partial<Completion>) =>
  ({ lines: 0, seenLines: 0, oldBaseLines: 0, items: 0, acknowledgedItems: 0, ...c }) as Completion;

void test("lines that are seen against an old base are neither seen nor unseen", () => {
  const c = completion({ lines: 77, seenLines: 47, oldBaseLines: 5, items: 3, acknowledgedItems: 1 });
  assert.equal(completionTitle(c), "47 seen · 5 base changed · 25 unseen · 1 of 3 other changes");
});

void test("base changed is left out when no line is", () => {
  assert.equal(completionTitle(completion({ lines: 10, seenLines: 4 })), "4 seen · 6 unseen · 0 of 0 other changes");
});
