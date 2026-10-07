import assert from "node:assert/strict";
import { test } from "node:test";
import type { Task } from "../generated/Task.ts";
import { fractionOf, headline } from "./taskHeadline.ts";

function task(id: number, done: number | null = null, total: number | null = null): Task {
  return {
    project: 1,
    id,
    kind: "scan",
    title: `task ${id}`,
    done,
    total,
    detail: null,
    state: { type: "running" },
  };
}

void test("nothing runs, nothing is said", () => {
  assert.equal(headline([]), undefined);
});

void test("the newest task is named and the others are counted", () => {
  assert.deepEqual(headline([task(1), task(2), task(3, 1, 4)]), { title: "task 3", fraction: 0.25, more: 2 });
  assert.deepEqual(headline([task(7)]), { title: "task 7", fraction: undefined, more: 0 });
});

void test("progress is determinate only with both numbers, and stays within the bar", () => {
  assert.equal(fractionOf(task(1, 3, null)), undefined);
  assert.equal(fractionOf(task(1, null, 10)), undefined);
  assert.equal(fractionOf(task(1, 0, 0)), undefined);
  assert.equal(fractionOf(task(1, 12, 10)), 1);
  assert.equal(fractionOf(task(1, 0, 10)), 0);
});
