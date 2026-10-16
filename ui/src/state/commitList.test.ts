import assert from "node:assert/strict";
import { test } from "node:test";
import type { CommitUnit } from "../generated/CommitUnit";
import { CommitList } from "./commitList.ts";

const unit = (id: string): CommitUnit => ({
  commit: { id, shortId: id.slice(0, 7), subject: id, author: "Lena", time: 0 },
  body: "",
  parents: ["p1"],
  kind: "normal",
  delta: { added: 1, removed: 0 },
  completion: {
    lines: 1,
    seenLines: 0,
    oldBaseLines: 0,
    hunks: 1,
    seenHunks: 0,
    items: 0,
    acknowledgedItems: 0,
    complete: false,
    seenPercent: 0,
    oldBasePercent: 0,
  },
});

/** A list over a scripted read: each call answers the next entry (the last one repeats). */
function scripted(...answers: ({ revision: number; commits: CommitUnit[] } | Error)[]) {
  const calls: number[] = [];
  const list = new CommitList(() => {
    const answer = answers[Math.min(calls.length, answers.length - 1)];
    calls.push(0);
    return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer);
  });
  return { list, reads: () => calls.length };
}

const ids = (units: CommitUnit[] | undefined) => units?.map((u) => u.commit.id);

void test("the commits of the held revision are given back without reading again", async () => {
  const { list, reads } = scripted({ revision: 4, commits: [unit("a"), unit("b")] });
  assert.deepEqual(ids(await list.at(4)), ["a", "b"]);
  assert.deepEqual(ids(await list.at(4)), ["a", "b"]);
  assert.equal(reads(), 1, "a matching revision starts no read");
});

void test("a read for a held older revision answers the new one", async () => {
  const { list, reads } = scripted(
    { revision: 3, commits: [unit("a")] },
    { revision: 4, commits: [unit("a"), unit("b")] },
  );
  await list.at(3);
  assert.deepEqual(ids(await list.at(4)), ["a", "b"]);
  assert.equal(reads(), 2);
});

void test("a list held for a newer revision is nothing, so the caller treats the view as outdated", async () => {
  const { list, reads } = scripted({ revision: 4, commits: [unit("a"), unit("b")] });
  await list.at(4);
  assert.equal(await list.at(3), undefined);
  assert.equal(reads(), 1, "a newer-held list is not read again");
});

void test("a read that failed reaches its caller, and the next call reads again", async () => {
  const { list, reads } = scripted(new Error("the socket dropped"), { revision: 4, commits: [unit("a")] });
  await assert.rejects(list.at(4), /socket dropped/);
  assert.deepEqual(ids(await list.at(4)), ["a"]);
  assert.equal(reads(), 2);
});

void test("the commits are read again so a held revision's progress can move on", async () => {
  const { list, reads } = scripted(
    { revision: 4, commits: [unit("a")] },
    { revision: 4, commits: [unit("a"), unit("b")] },
  );
  await list.at(4);
  const got = await list.reload();
  assert.deepEqual(ids(got.units), ["a", "b"]);
  assert.equal(got.revision, 4);
  assert.equal(reads(), 2);
});

void test("callers asking while a read runs join it instead of starting another", async () => {
  const releases: ((answer: { revision: number; commits: CommitUnit[] }) => void)[] = [];
  let reads = 0;
  const list = new CommitList(() => {
    reads++;
    return new Promise((resolve) => releases.push(resolve));
  });
  const first = list.at(4);
  const second = list.at(4);
  for (const release of releases) release({ revision: 4, commits: [unit("a")] });
  assert.deepEqual(ids(await first), ["a"]);
  assert.deepEqual(ids(await second), ["a"]);
  assert.equal(reads, 1);
});

/** A list over reads the caller releases by hand, in the order they started. */
function deferred() {
  const releases: ((answer: { revision: number; commits: CommitUnit[] }) => void)[] = [];
  const list = new CommitList(() => new Promise((resolve) => releases.push(resolve)));
  return { list, releases };
}

void test("a reload during a read in flight starts its own read and takes the newer answer", async () => {
  const { list, releases } = deferred();
  const first = list.at(4);
  const second = list.reload();
  assert.equal(releases.length, 2, "the reload joined the read that began before it");
  releases[1]({ revision: 4, commits: [unit("b")] });
  assert.deepEqual(ids((await second).units), ["b"]);
  releases[0]({ revision: 4, commits: [unit("a")] });
  assert.deepEqual(ids(await first), ["a"]);
  assert.deepEqual(ids(await list.at(4)), ["b"], "the newer read's answer is the held one");
});

void test("an older read that finishes after a newer one does not replace the held list", async () => {
  const { list, releases } = deferred();
  const older = list.reload();
  const newer = list.reload();
  releases[1]({ revision: 5, commits: [unit("new")] });
  await newer;
  releases[0]({ revision: 4, commits: [unit("old")] });
  await older;
  // The held list is of the newer revision: asking for the older one is outdated, not the old list.
  assert.equal(await list.at(4), undefined);
  assert.equal(releases.length, 2, "the held list answered without a read");
  assert.deepEqual(ids(await list.at(5)), ["new"]);
});
