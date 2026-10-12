import assert from "node:assert/strict";
import { test } from "node:test";
import { provideActions, reviewActions } from "./reviewActions.ts";

void test("the review's commands do nothing, and find nothing to do, while no review is mounted", () => {
  assert.equal(reviewActions.toggleSeen(), false);
  assert.equal(reviewActions.revealFile(), false);
  assert.doesNotThrow(() => reviewActions.step(1));
  assert.doesNotThrow(() => reviewActions.stepTab(-1));
});

void test("what a mounted screen and its controller supply is what the commands do, until they go", () => {
  const calls: string[] = [];
  const stopController = provideActions({ step: (d) => calls.push(`step ${d}`), toggleSeen: () => true });
  const stopScreen = provideActions({ stepTab: (d) => calls.push(`tab ${d}`), revealFile: () => true });
  reviewActions.step(1);
  reviewActions.stepTab(-1);
  assert.equal(reviewActions.toggleSeen(), true);
  assert.equal(reviewActions.revealFile(), true);
  assert.deepEqual(calls, ["step 1", "tab -1"]);

  stopScreen();
  assert.equal(reviewActions.revealFile(), false);
  assert.equal(reviewActions.toggleSeen(), true);
  stopController();
  assert.equal(reviewActions.toggleSeen(), false);
});
