import assert from "node:assert/strict";
import { test } from "node:test";
import type { PendingVote } from "../../generated/PendingVote.ts";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import {
  blockerText,
  draftsToSubmit,
  postedNote,
  stateLabel,
  stateTone,
  submitBlock,
  waitingToPost,
} from "./pullRequest.ts";

const message = (state: ThreadMessage["state"]): ThreadMessage => ({
  id: 1,
  author: "me",
  body: "x",
  at: 0,
  mine: true,
  avatar: null,
  state,
  editedAt: null,
  conflict: null,
});
const [draft, posted, posting] = [{ type: "draft" }, { type: "posted" }, { type: "posting" }] as const;
const failed = { type: "failed", error: "locked" } as const;
const thread = (messages: ThreadMessage[], onPlatform: boolean): Thread => ({
  id: 1,
  kind: "changeset",
  side: null,
  path: null,
  original: null,
  location: null,
  targetLine: null,
  anchor: null,
  messages,
  status: "open",
  platform: onPlatform ? { posted: false, resolvable: true, resolvedBy: null } : null,
});

void test("the drafts and failed messages of a platform's threads wait for the review, not those on their way", () => {
  const threads = [
    thread([message(draft), message(posted), message(failed), message(posting)], true),
    thread([message(draft)], false),
  ];
  assert.equal(draftsToSubmit(threads), 2);
});

void test("a signed out or expired platform blocks submitting and says what to do", () => {
  assert.equal(submitBlock({ type: "ok" }), undefined);
  assert.equal(submitBlock({ type: "failed", message: "offline" }), undefined);
  assert.match(submitBlock({ type: "signedOut" }) ?? "", /Sign in/);
  assert.match(submitBlock({ type: "tokenExpired", message: "" }) ?? "", /sign in again/);
});

void test("a post sends the waiting comments and the staged vote unless that is on its way already", () => {
  const threads = [thread([message(draft), message(failed)], true)];
  const vote = (state: PendingVote["state"]): PendingVote => ({ verdict: "approved", state });
  assert.equal(waitingToPost(threads, null), 2);
  assert.equal(waitingToPost(threads, vote(draft)), 3);
  assert.equal(waitingToPost(threads, vote(failed)), 3);
  assert.equal(waitingToPost(threads, vote(posting)), 2);
  assert.equal(waitingToPost([], vote(draft)), 1, "a vote alone can be posted");
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
