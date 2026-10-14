import assert from "node:assert/strict";
import { test } from "node:test";
import { Debounced } from "./debounced.ts";

void test("only the last call of a burst runs, after its own wait", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const ran: string[] = [];
  const wait = new Debounced();
  wait.schedule(100, () => ran.push("first"));
  t.mock.timers.tick(60);
  wait.schedule(100, () => ran.push("second"));
  t.mock.timers.tick(60);
  assert.deepEqual(ran, []);
  assert.equal(wait.pending, true);
  t.mock.timers.tick(40);
  assert.deepEqual(ran, ["second"]);
  assert.equal(wait.pending, false);
});

void test("a cancelled call does not run, and the next one does", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const ran: string[] = [];
  const wait = new Debounced();
  wait.schedule(50, () => ran.push("dropped"));
  wait.cancel();
  assert.equal(wait.pending, false);
  t.mock.timers.tick(100);
  assert.deepEqual(ran, []);
  wait.schedule(50, () => ran.push("kept"));
  t.mock.timers.tick(50);
  assert.deepEqual(ran, ["kept"]);
});
