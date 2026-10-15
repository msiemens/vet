import assert from "node:assert/strict";
import { test } from "node:test";
import { blockerText, postedNote, stateLabel, stateTone, submitBlock } from "./pullRequest.ts";

void test("a signed out or expired platform blocks submitting and says what to do", () => {
  assert.equal(submitBlock({ type: "ok" }), undefined);
  assert.equal(submitBlock({ type: "failed", message: "offline" }), undefined);
  assert.match(submitBlock({ type: "signedOut" }) ?? "", /Sign in/);
  assert.match(submitBlock({ type: "tokenExpired", message: "" }) ?? "", /sign in again/);
});

void test("a post tells how many comments went through and that the vote was cast", () => {
  assert.equal(postedNote({ posted: 2, skipped: 0, voteCast: false }), "2 comments posted");
  assert.equal(postedNote({ posted: 1, skipped: 0, voteCast: true }), "1 comment posted, vote cast");
  assert.equal(postedNote({ posted: 0, skipped: 0, voteCast: true }), "Vote cast");
  assert.match(postedNote({ posted: 1, skipped: 2, voteCast: true }), /^1 comment posted, vote cast\. 2 drafts wait/);
});

void test("a closed pull request is flagged, a merged one has its own colour, an open one is well", () => {
  assert.equal(stateTone("open"), "ok");
  assert.equal(stateTone("closed"), "warn");
  assert.equal(stateTone("merged"), "note");
});

void test("the pull request's state and what blocks it read as words", () => {
  assert.equal(stateLabel({ state: "open", draft: true }), "Draft");
  assert.equal(stateLabel({ state: "merged", draft: false }), "Merged");
  assert.equal(blockerText({ type: "threads", count: 1 }), "1 unresolved thread");
  assert.equal(blockerText({ type: "threads", count: 3 }), "3 unresolved threads");
});
