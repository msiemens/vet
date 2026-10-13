import assert from "node:assert/strict";
import { test } from "node:test";
import { provideActions, reviewActions } from "./reviewActions.ts";

void test("the review's commands do nothing, and find nothing to do, while no review is mounted", () => {
  assert.equal(reviewActions.toggleSeen(), false);
  assert.equal(reviewActions.revealFile(), false);
});

void test("closing a tab names the commit it is pinned to, so the pinned tab closes and not the file's other tab", () => {
  const calls: unknown[][] = [];
  const stop = provideActions({
    closeTab: (...args) => calls.push(["close", ...args]),
    closeOtherTabs: (...args) => calls.push(["others", ...args]),
  });
  reviewActions.closeTab("file", "a.rs", "c0ffee");
  reviewActions.closeOtherTabs("file", "a.rs", "c0ffee");
  stop();
  assert.deepEqual(calls, [
    ["close", "file", "a.rs", "c0ffee"],
    ["others", "file", "a.rs", "c0ffee"],
  ]);
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
