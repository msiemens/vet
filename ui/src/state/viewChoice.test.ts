import assert from "node:assert/strict";
import { test } from "node:test";
import type { View } from "../generated/View.ts";
import { RequestError } from "../lib/requestError.ts";
import {
  type Ask,
  FIRST_CHOICE,
  type Said,
  type ViewChoice,
  askOf,
  commitSettled,
  limit,
  onLens,
  settleView,
  showChangeset,
  showCommit,
  showRange,
  viewOf,
} from "./viewChoice.ts";

const RANGE = { from: 1, to: 2 };
const SQUASH: Said = { latest: 3, lens: "squash", commits: 3 };
const BY_COMMIT: Said = { latest: 3, lens: "perCommit", commits: 3 };
const notFound = () => new RequestError("notFound", "no such version");

/** Settles `ask` against a backend that answers each view with `answers` (a manifest or the error to reject with). */
function settle(
  ask: Ask,
  answers: (view: View) => Said | Error,
  { last = 0, seed }: { last?: number; seed?: Said } = {},
) {
  const reads: View[] = [];
  const adopted: Said[] = [];
  let seeded = seed;
  const run = settleView<Said>(ask, last, {
    read: (view) => {
      reads.push(view);
      const answer = answers(view);
      return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer);
    },
    said: (m) => m,
    seed: () => {
      const taken = seeded;
      seeded = undefined;
      return taken;
    },
    onChangeset: (m) => adopted.push(m),
  });
  return { run, reads, adopted };
}

const isChangeset = (view: View) => view.type === "changeset";

void test("a commit the backend does not find gives way to the changeset and is told", async () => {
  const { run, reads } = settle({ commit: 4 }, (view) => (isChangeset(view) ? SQUASH : notFound()));
  const settled = await run;
  assert.deepEqual(settled.ask, {});
  assert.equal(settled.refused.length, 1);
  assert.deepEqual(
    reads.map((v) => v.type),
    ["commit", "changeset"],
  );
});

void test("a range of versions that are gone gives way to the changeset and is told", async () => {
  const { run } = settle({ range: RANGE }, (view) => (isChangeset(view) ? SQUASH : notFound()));
  const settled = await run;
  assert.equal(settled.ask.range, undefined);
  assert.equal(settled.refused.length, 1);
});

void test("a refusal of another kind rejects and keeps the range", async () => {
  const { run } = settle({ range: RANGE }, () => new RequestError("invalid", "an interdiff compares"));
  await assert.rejects(run, /an interdiff compares/);
});

void test("the changeset not found rejects", async () => {
  const { run } = settle({}, () => notFound());
  await assert.rejects(run, /no such version/);
});

void test("a range over every push of an open review reads the changeset", async () => {
  const { run, reads, adopted } = settle({ range: { from: 0, to: 3 } }, (view) =>
    isChangeset(view) ? SQUASH : BY_COMMIT,
  );
  const settled = await run;
  assert.equal(settled.ask.range, undefined);
  assert.equal(settled.refused.length, 0);
  assert.equal(reads.length, 2);
  assert.deepEqual(adopted, [SQUASH], "only the changeset's manifest is the changeset's");
});

void test("by commit, asking the changeset reads the last commit, clamped, and hands over the changeset", async () => {
  const { run, adopted } = settle({}, () => BY_COMMIT, { last: 7 });
  const settled = await run;
  assert.deepEqual(settled.ask, { commit: 2 });
  assert.deepEqual(adopted, [BY_COMMIT]);
});

void test("in the squash lens a commit asked for reads the changeset", async () => {
  const { run, reads } = settle({ commit: 1 }, () => SQUASH);
  const settled = await run;
  assert.equal(settled.ask.commit, undefined);
  assert.deepEqual(
    reads.map((v) => v.type),
    ["commit", "changeset"],
  );
});

void test("a commit refused again and again settles on the changeset after three re-asks", async () => {
  const { run, reads } = settle({ commit: 1 }, (view) => (isChangeset(view) ? BY_COMMIT : notFound()), { last: 1 });
  const settled = await run;
  assert.deepEqual(settled.ask, {});
  assert.equal(reads.length, 4);
  assert.equal(settled.refused.length, 2);
});

void test("the seed is used once, for the changeset only", async () => {
  const seed = { ...SQUASH, commits: 9 };
  const withRange = settle({ range: RANGE }, () => SQUASH, { seed });
  assert.equal((await withRange.run).manifest, SQUASH);
  assert.equal(withRange.reads.length, 1);

  const changeset = settle({}, () => SQUASH, { seed });
  assert.equal((await changeset.run).manifest, seed);
  assert.equal(changeset.reads.length, 0);
});

void test("a settled read does not bring back a range cleared while it ran", () => {
  const asked: ViewChoice = showRange(FIRST_CHOICE, RANGE);
  const from = askOf(asked);
  const cleared = showRange(asked, undefined);
  assert.equal(commitSettled(cleared, from, {}), cleared);
  const other = showRange(asked, { from: 1, to: 2 });
  assert.equal(commitSettled(other, from, {}), other, "a range chosen since is another one, whatever it holds");
});

void test("a settled read takes its fallback when the choice did not move", () => {
  const asked = showRange(FIRST_CHOICE, RANGE);
  const settled = commitSettled(asked, askOf(asked), {});
  assert.equal(settled.range, undefined);
  const byCommit = showCommit(FIRST_CHOICE, 5);
  assert.equal(commitSettled(byCommit, askOf(byCommit), {}).commit, undefined);
  assert.equal(commitSettled(byCommit, askOf(byCommit), { commit: 2 }).commit, 2);
});

void test("by commit drops a range and goes to the last commit", () => {
  const choice = onLens({ ...showRange(FIRST_CHOICE, RANGE), last: 2 }, "perCommit");
  assert.equal(choice.range, undefined);
  assert.equal(choice.commit, 2);
  assert.deepEqual(viewOf(askOf(choice)), { type: "commit", index: 2 });
});

void test("the squash lens keeps a range on view", () => {
  const ranged = showRange(FIRST_CHOICE, RANGE);
  assert.equal(onLens(ranged, "squash"), ranged);
  const byCommit = showCommit(FIRST_CHOICE, 1);
  assert.equal(onLens(byCommit, "squash").commit, undefined);
  assert.equal(onLens(byCommit, "squash").last, 1, "the commit shows again by commit");
});

void test("a range clears the base-changed limit; no range keeps the commit", () => {
  const limited = limit(showCommit(FIRST_CHOICE, 3), new Set(["a.txt"]));
  assert.equal(showRange(limited, RANGE).limitedTo, undefined);
  const unranged = showRange(limited, undefined);
  assert.equal(unranged.commit, 3);
  assert.deepEqual([...(unranged.limitedTo ?? [])], ["a.txt"]);
  assert.equal(showChangeset(limited).commit, undefined);
});
