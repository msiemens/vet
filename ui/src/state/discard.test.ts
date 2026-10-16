import assert from "node:assert/strict";
import { test } from "node:test";
import { discardTitle, whenDiscarded } from "./discard.ts";

void test("the title counts the comments only when there are several", () => {
  assert.equal(discardTitle(1), "Discard your comment?");
  assert.equal(discardTitle(2), "Discard 2 comments?");
  assert.equal(discardTitle(5), "Discard 5 comments?");
});

/** `whenDiscarded` with the answers given; `went` says whether it went ahead and `asked` what it was asked. */
function attempt(count: number, answer: boolean) {
  const asked: number[] = [];
  let went = false;
  const done = whenDiscarded(
    count,
    (n) => (asked.push(n), Promise.resolve(answer)),
    () => void (went = true),
  );
  return { done, asked, went: () => went };
}

void test("with no text in a box it goes ahead at once and asks nothing", async () => {
  const { done, asked, went } = attempt(0, false);
  assert.equal(went(), true, "it ran before anything was awaited");
  await done;
  assert.deepEqual(asked, []);
});

void test("with text it asks how many boxes hold it, and goes ahead only on Discard", async () => {
  const discarded = attempt(2, true);
  assert.equal(discarded.went(), false, "it waits for the answer");
  await discarded.done;
  assert.deepEqual(discarded.asked, [2]);
  assert.equal(discarded.went(), true);

  const kept = attempt(1, false);
  await kept.done;
  assert.deepEqual(kept.asked, [1]);
  assert.equal(kept.went(), false, "Keep editing cancels it");
});
