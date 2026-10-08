import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../../generated/Thread.ts";
import type { ThreadMessage } from "../../generated/ThreadMessage.ts";
import { blockerText, draftsToSubmit, stateLabel, submitBlock } from "./pullRequest.ts";

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

void test("the pull request's state and what blocks it read as words", () => {
  assert.equal(stateLabel({ state: "open", draft: true }), "Draft");
  assert.equal(stateLabel({ state: "merged", draft: false }), "Merged");
  assert.equal(blockerText({ type: "threads", count: 1 }), "1 unresolved thread");
  assert.equal(blockerText({ type: "threads", count: 3 }), "3 unresolved threads");
});
