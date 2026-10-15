import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { Completion } from "../../generated/Completion";
import type { Reviewer } from "../../generated/Reviewer";
import type { Thread } from "../../generated/Thread";
import { metaOf, reasonOf, repliesForYou, reviewSummary, reviewersSummary } from "./queuePanel.ts";

type Pull = Parameters<typeof reasonOf>[0]["pull"];

const pull = (over: Partial<Pull> = {}): Pull => ({
  status: "blocked",
  ci: "success",
  changesRequested: [],
  unresolvedThreads: 0,
  ...over,
});
const reason = (over: Partial<Parameters<typeof reasonOf>[0]> = {}) =>
  reasonOf({ pull: pull(), platform: "github", failed: [], unread: 0, ...over });

void test("a blocked pull request names the failed checks and the open threads", () => {
  const r = reason({ failed: ["build"], pull: pull({ unresolvedThreads: 2 }) });
  assert.deepEqual(r.parts, [{ bold: "build", text: " failed" }, { text: "2 unresolved threads" }]);
  assert.equal(r.tone, "bad");
  assert.equal(r.protection, false);
});

void test("a pull request on GitHub that is blocked for no reason Vet knows is held by branch protection", () => {
  assert.equal(reason().protection, true);
  assert.equal(reason({ platform: "azureDevOps" }).protection, false);
  assert.equal(reason({ platform: undefined }).protection, false, "an unknown platform claims nothing");
  assert.equal(reason({ failed: ["lint"] }).protection, false);
  assert.equal(reason({ pull: pull({ ci: "failure" }) }).protection, false, "failing checks are a reason");
});

void test("a pull request that is not blocked says where it stands", () => {
  const r = reasonOf({
    pull: pull({ status: "waitingForYou", ci: "success" }),
    info: { reviewers: [{ login: "t", name: "Tobias Lang", state: "approved" }] },
    platform: "github",
    failed: [],
    unread: 2,
  });
  assert.deepEqual(
    r.parts.map((p) => p.text),
    ["Checks pass", "Tobias Lang approved", "2 replies for you"],
  );
  assert.equal(r.tone, "muted");
  assert.equal(r.protection, false);
});

const open = (over: Partial<{ lastReviewed: number; version: number }> = {}) =>
  ({ state: "open", version: 4, startedAt: 0, lastReviewed: 2, ...over }) as const;
const completion = (over: Partial<Completion> = {}): Completion => ({
  lines: 2740,
  seenLines: 1042,
  oldBaseLines: 164,
  hunks: 0,
  seenHunks: 0,
  flaggedHunks: 0,
  items: 0,
  acknowledgedItems: 0,
  complete: false,
  seenPercent: 38.04,
  oldBasePercent: 6,
  ...over,
});
const input = (over: Partial<Parameters<typeof reviewSummary>[0]> = {}): Parameters<typeof reviewSummary>[0] => ({
  review: { state: "none" },
  asked: false,
  noun: ["push", "pushes"],
  ...over,
});

void test("a review not started says how big the work is", () => {
  const s = reviewSummary(input({ completion: completion({ lines: 2737 }), files: 34 }));
  assert.equal(s.state, "Not started");
  assert.equal(s.detail, "2,737 changed lines in 34 files");
  assert.equal(s.percent, undefined);
});

void test("a review in progress says how far it is, with what was seen against an older base", () => {
  const s = reviewSummary(input({ review: open(), completion: completion() }));
  assert.equal(s.state, "In progress");
  assert.equal(s.detail, "1,042 of 2,740 lines seen");
  assert.equal(s.percent, 38);
  assert.equal(s.oldBaseNote, "164 lines were seen against an older base.");
});

void test("the pushes since the last review count their files once and the lines seen again", () => {
  const s = reviewSummary(
    input({
      review: open(),
      completion: completion(),
      changesSince: { from: 2, to: 5, files: 2 },
    }),
  );
  assert.equal(
    s.pushes,
    "3 pushes since you last reviewed (v2 → v5): 2 files changed, and 164 lines you had seen changed again",
  );
  assert.equal(reviewSummary(input({ review: open({ lastReviewed: 4 }), completion: completion() })).pushes, undefined);
});

void test("the base is named with the branch it is compared against", () => {
  const base = { branch: "develop", shortId: "44ef7e8" };
  assert.equal(reviewSummary(input({ base })).base, "Base develop · 44ef7e8");
  assert.equal(reviewSummary(input({ base: { shortId: "44ef7e8" } })).base, "Base 44ef7e8");
});

void test("a finished complete review fills the meter", () => {
  const s = reviewSummary(input({ review: { state: "finished", complete: true, startedAt: 0, finishedAt: 0 } }));
  assert.deepEqual([s.detail, s.seen, s.oldBase], ["complete", 100, 0]);
});

const reviewer = (name: string, state: Reviewer["state"]): Reviewer => ({ login: name, name, state });

void test("folded reviewers name who decided and count who waits", () => {
  assert.equal(
    reviewersSummary([reviewer("Tobias", "approved"), reviewer("Anna", "waiting"), reviewer("Lena", "waiting")]),
    "Tobias ✓ · 2 waiting",
  );
  assert.equal(reviewersSummary([reviewer("Jonas", "changesRequested")]), "Jonas ✗");
  assert.equal(reviewersSummary([reviewer("Anna", "commented")]), "");
});

const thread = (...mine: boolean[]): Thread =>
  ({ messages: mine.map((m, i) => ({ id: i, mine: m, at: 10 + i })) }) as unknown as Thread;

void test("replies for you are the unread messages of threads the viewer is in", () => {
  const unread = (m: { at: number }) => m.at >= 11;
  assert.equal(repliesForYou([thread(true, false, false), thread(false, false)], unread), 2);
  assert.equal(repliesForYou([], unread), 0);
});

void test("the working tree's line says where its changes live", () => {
  const tree: ChangesetRow = {
    key: { type: "workingTree" },
    title: "Working tree",
    checkedOut: false,
    branch: "develop",
    openThreads: 0,
    review: { state: "none" },
  };
  assert.deepEqual(metaOf(tree, 4, Date.now()), [
    { type: "text", text: "Live changes on" },
    { type: "mono", text: "develop" },
    { type: "text", text: "4 files, not committed" },
  ]);
});
