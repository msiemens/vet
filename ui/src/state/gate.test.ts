import assert from "node:assert/strict";
import { test } from "node:test";
import { Gate } from "./gate.ts";

void test("no more tasks than the limit run at once, and the rest follow in order", async () => {
  const gate = new Gate(2);
  let running = 0;
  let peak = 0;
  const started: number[] = [];
  const task = (n: number) =>
    gate.run(async () => {
      started.push(n);
      running++;
      peak = Math.max(peak, running);
      await new Promise((r) => setTimeout(r, 5));
      running--;
      return n * 10;
    });
  const results = await Promise.all([1, 2, 3, 4, 5].map(task));
  assert.deepEqual(results, [10, 20, 30, 40, 50]);
  assert.equal(peak, 2);
  assert.deepEqual(started, [1, 2, 3, 4, 5]);
});

void test("a failing task gives its place to the next one", async () => {
  const gate = new Gate(1);
  const failing = gate.run(() => Promise.reject(new Error("no")));
  const next = gate.run(() => Promise.resolve("ran"));
  await assert.rejects(failing, /no/);
  assert.equal(await next, "ran");
});
