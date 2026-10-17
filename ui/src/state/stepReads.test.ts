import assert from "node:assert/strict";
import { test } from "node:test";
import type { CheckPhase } from "../generated/CheckPhase.ts";
import type { CheckRun } from "../generated/CheckRun.ts";
import { type StepsRead, readSteps } from "./stepReads.ts";

/** A new record each call, as every push of the checks brings. */
const run = (id: string, phase: CheckPhase = "running"): CheckRun => ({
  id,
  name: id,
  state: "pending",
  phase,
  log: true,
});

const held = (r: CheckRun): StepsRead => ({ job: r.id, run: r });

void test("a job whose steps were not read is read", () => {
  assert.equal(readSteps(undefined, run("a"), false, false), true);
  assert.equal(readSteps(undefined, run("a", "finished"), true, false), true);
});

void test("a finished job is read once, whatever records later pushes bring", () => {
  const read = run("a", "finished");
  assert.equal(readSteps(held(read), run("a", "finished"), true, false), false);
});

void test("a running job is read again for each record a push brings, even one equal to the last", () => {
  const read = run("a");
  assert.equal(readSteps(held(read), run("a"), false, false), true);
});

void test("the record already read is not read again", () => {
  const read = run("a");
  assert.equal(readSteps(held(read), read, false, false), false);
});

void test("another job is read", () => {
  assert.equal(readSteps(held(run("a", "finished")), run("b", "finished"), true, false), true);
});

void test("a job whose last read failed is read again, finished or not", () => {
  const read = run("a", "finished");
  assert.equal(readSteps(held(read), read, true, true), true);
  const going = run("a");
  assert.equal(readSteps(held(going), going, false, true), true);
});
