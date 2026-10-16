import assert from "node:assert/strict";
import { test } from "node:test";
import { Lifetime, LifetimeEnded } from "./lifetime.ts";

void test("disposing runs the cleanups newest first, and once", () => {
  const lifetime = new Lifetime();
  const ran: string[] = [];
  lifetime.add(() => ran.push("first"));
  lifetime.add(() => ran.push("second"));
  lifetime.dispose();
  lifetime.dispose();
  assert.deepEqual(ran, ["second", "first"]);
  assert.equal(lifetime.alive, false);
});

void test("a cleanup added after the end runs at once", () => {
  const lifetime = new Lifetime();
  lifetime.dispose();
  let ran = false;
  lifetime.add(() => (ran = true));
  assert.equal(ran, true);
});

void test("a step that checks the lifetime stops after the end", () => {
  const lifetime = new Lifetime();
  lifetime.check();
  lifetime.dispose();
  assert.throws(() => lifetime.check(), LifetimeEnded);
});
