import assert from "node:assert/strict";
import { test } from "node:test";
import { HoldTimer } from "./holdTimer.ts";

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

void test("fires once the time has passed", async () => {
  let fired = 0;
  new HoldTimer(() => fired++, 10);
  await wait(30);
  assert.equal(fired, 1);
});

void test("a held timer does not fire until released, then waits the full time again", async () => {
  let fired = 0;
  const timer = new HoldTimer(() => fired++, 40);
  await wait(20);
  timer.hold();
  await wait(60);
  assert.equal(fired, 0);
  timer.release();
  await wait(20);
  assert.equal(fired, 0);
  await wait(50);
  assert.equal(fired, 1);
});

void test("a stopped timer never fires, even when released", async () => {
  let fired = 0;
  const timer = new HoldTimer(() => fired++, 10);
  timer.stop();
  timer.release();
  await wait(30);
  assert.equal(fired, 0);
});

void test("releasing after it fired does not fire it again", async () => {
  let fired = 0;
  const timer = new HoldTimer(() => fired++, 10);
  await wait(30);
  timer.release();
  await wait(30);
  assert.equal(fired, 1);
});
