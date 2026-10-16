import assert from "node:assert/strict";
import { test } from "node:test";
import type { PullRequestRow } from "../generated/PullRequestRow";
import type { PullRequestUpdate } from "../generated/PullRequestUpdate";
import { type QueueActions, queueEvent, withDetails } from "./queueEvents.ts";

function row(number: number): PullRequestRow {
  return {
    key: { type: "pullRequest", remote: "github.com/acme/app", number },
    title: `PR ${number}`,
    author: "ada",
    authorName: "Ada",
    sourceBranch: "feature",
    targetBranch: "main",
    comments: 0,
    draft: false,
    changesRequested: [],
    waitingForMe: false,
    unresolvedThreads: 0,
    updatedAt: 1,
    review: { state: "none" },
    url: `https://github.com/acme/app/pull/${number}`,
  };
}

function update(number: number): PullRequestUpdate {
  return {
    key: { type: "pullRequest", remote: "github.com/acme/app", number },
    status: "changesRequested",
    unresolvedThreads: 2,
    changesRequested: [{ reviewer: "Linus", how: "requested" }],
  };
}

void test("the event that details were read patches the rows and lists nothing again", () => {
  const did: string[] = [];
  const actions: QueueActions = {
    load: () => did.push("load"),
    loadPullRequests: () => did.push("listPullRequests"),
    takeLines: () => did.push("takeLines"),
    takeDetails: (changed) => did.push(`takeDetails ${changed.length}`),
    follow: () => did.push("follow"),
  };

  queueEvent({ type: "pullRequestsChanged", project: 1, changed: [update(7)] }, actions);

  assert.deepEqual(did, ["takeDetails 1"]);
});

void test("opening a changeset and starting a review list the local rows and no pull requests", () => {
  const did: string[] = [];
  const actions: QueueActions = {
    load: () => did.push("load"),
    loadPullRequests: () => did.push("listPullRequests"),
    takeLines: () => did.push("takeLines"),
    takeDetails: () => did.push("takeDetails"),
    follow: () => did.push("follow"),
  };

  queueEvent({ type: "changesetOpened", project: 1, revision: 1, changeset: { type: "workingTree" } }, actions);
  queueEvent({ type: "reviewStarted", project: 1, review: { state: "open", version: 1, updatedAt: 0 } }, actions);

  assert.deepEqual(did, ["load", "load"]);
});

void test("the details that were read show in the row they changed and in no other", () => {
  const rows = [row(7), row(8)];

  const patched = withDetails(rows, [update(7)]);

  assert.deepEqual(
    {
      status: patched[0].status,
      unresolvedThreads: patched[0].unresolvedThreads,
      changesRequested: patched[0].changesRequested,
    },
    {
      status: "changesRequested",
      unresolvedThreads: 2,
      changesRequested: [{ reviewer: "Linus", how: "requested" }],
    },
  );
  assert.equal(patched[1], rows[1], "a row the details did not change is left as it is");
});

void test("a status the details left out is cleared from the row", () => {
  const shown = { ...row(7), status: "blocked" } as PullRequestRow;

  const patched = withDetails([shown], [{ ...update(7), status: undefined }]);

  assert.equal(patched[0].status, undefined);
});
