import assert from "node:assert/strict";
import { test } from "node:test";
import { IntentQueue } from "./intentQueue.ts";

void test("an operation runs at once while the controller is ready", () => {
  const log: string[] = [];
  const queue = new IntentQueue<string>();
  queue.when((value) => log.push(`act ${value}`), "controller");
  assert.deepEqual(log, ["act controller"]);
});

void test("an operation waits for the controller and runs when it comes, in order", () => {
  const log: string[] = [];
  const queue = new IntentQueue<string>();
  queue.when((value) => log.push(`first ${value}`), undefined);
  queue.when((value) => log.push(`second ${value}`), undefined);
  assert.deepEqual(log, []);
  queue.ready("controller");
  assert.deepEqual(log, ["first controller", "second controller"]);
});

void test("one ready controller runs each held operation once", () => {
  const log: string[] = [];
  const queue = new IntentQueue<string>();
  queue.when((value) => log.push(value), undefined);
  queue.ready("a");
  queue.ready("b");
  assert.deepEqual(log, ["a"]);
});

void test("what was held when the screen went away is dropped", () => {
  const log: string[] = [];
  const queue = new IntentQueue<string>();
  queue.when((value) => log.push(value), undefined);
  queue.clear();
  queue.ready("controller");
  assert.deepEqual(log, []);
});
