import assert from "node:assert/strict";
import { test } from "node:test";
import type { Task } from "../generated/Task.ts";
import { TaskBoard } from "./taskBoard.ts";

function task(
  id: number,
  state: Task["state"] = { type: "running" },
  title = `task ${id}`,
  kind: Task["kind"] = "scan",
): Task {
  return { project: 1, id, kind, title, done: null, total: null, detail: null, state };
}

const ids = (tasks: Task[]) => tasks.map((t) => t.id);

void test("a task is shown once it has run for the delay, and not after it ended", () => {
  const board = new TaskBoard();
  board.apply(task(1), 1000, 300);
  assert.deepEqual(ids(board.shown(1299)), []);
  assert.equal(board.nextShown(1000), 300);
  assert.deepEqual(ids(board.shown(1300)), [1]);
  assert.equal(board.nextShown(1300), undefined);

  board.apply(task(1, { type: "finished" }), 1400, 300);
  assert.deepEqual(ids(board.shown(2000)), []);
});

void test("a step that ends within the delay is never shown", () => {
  const board = new TaskBoard();
  board.apply(task(1), 0, 300);
  board.apply(task(1, { type: "finished" }), 120, 300);
  assert.deepEqual(ids(board.shown(10_000)), []);
  assert.equal(board.nextShown(120), undefined);
});

void test("an update keeps the time the task is shown from and replaces its content", () => {
  const board = new TaskBoard();
  board.apply(task(1), 0, 300);
  board.apply({ ...task(1), done: 5, total: 10, title: "renamed" }, 250, 300);
  assert.deepEqual(ids(board.shown(300)), [1]);
  assert.deepEqual(
    board.shown(300).map((t) => [t.title, t.done]),
    [["renamed", 5]],
  );
});

void test("shown tasks come oldest first, however they arrived", () => {
  const board = new TaskBoard();
  board.apply(task(3), 0, 0);
  board.apply(task(1), 0, 0);
  board.apply(task(2), 0, 0);
  assert.deepEqual(ids(board.shown(0)), [1, 2, 3]);
});

void test("the backend's list replaces what was known and keeps the time of tasks it still lists", () => {
  const board = new TaskBoard();
  board.apply(task(1), 0, 300);
  board.apply(task(2), 0, 300);
  board.replace([task(2), task(4)], 400, 300);
  assert.deepEqual(ids(board.shown(300)), [2]);
  assert.deepEqual(ids(board.shown(700)), [2, 4]);
});

void test("a failed sync or checks refresh stays until a task of its kind finishes", () => {
  const board = new TaskBoard();
  const failed = { type: "failed", message: "offline" } as const;
  board.apply(task(1, failed, "Pull request sync", "sync"), 0, 0);
  board.apply(task(2, failed, "Checks refresh", "checks"), 0, 0);
  assert.deepEqual(ids(board.failed()), [1, 2]);
  assert.deepEqual(ids(board.shown(10_000)), []);

  board.apply(task(3, { type: "running" }, "Pull request sync", "sync"), 10, 0);
  board.apply(task(3, { type: "finished" }, "Pull request sync", "sync"), 11, 0);
  assert.deepEqual(ids(board.failed()), [2]);

  board.apply(task(4, failed, "Pull request sync", "sync"), 20, 0);
  board.apply(task(5, failed, "Pull request sync", "sync"), 30, 0);
  assert.deepEqual(ids(board.failed()), [2, 5], "a newer failure of a kind replaces the older");
});

void test("clearing the failures keeps the running tasks", () => {
  const board = new TaskBoard();
  board.apply(task(1, { type: "failed", message: "offline" }, "Pull request sync", "sync"), 0, 0);
  board.apply(task(2), 0, 0);
  board.clearFailures();
  assert.deepEqual(board.failed(), []);
  assert.deepEqual(ids(board.shown(0)), [2]);
});

void test("a failed task of another kind is not kept", () => {
  const board = new TaskBoard();
  board.apply(task(1, { type: "failed", message: "gone" }, "Fetching", "fetch"), 0, 0);
  assert.deepEqual(board.failed(), []);
});
