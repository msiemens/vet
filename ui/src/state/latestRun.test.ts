import assert from "node:assert/strict";
import { test } from "node:test";
import { abortError } from "../lib/abort.ts";
import { LatestRun } from "./latestRun.ts";

/** A request that answers when told, and rejects like a cancelled one when its signal aborts. */
function pending() {
  const signals: AbortSignal[] = [];
  const answers: ((value: string) => void)[] = [];
  const ask = (signal: AbortSignal) => {
    signals.push(signal);
    return new Promise<string>((resolve, reject) => {
      answers.push(resolve);
      signal.addEventListener("abort", () => reject(abortError()));
    });
  };
  return { ask, signals, answers };
}

void test("asking again cancels the run before it, and only the later answer is taken", async () => {
  const { ask, signals, answers } = pending();
  const run = new LatestRun<string>();
  const first = run.run(ask);
  const second = run.run(ask);
  assert.equal(signals[0].aborted, true);
  assert.equal(signals[1].aborted, false);
  answers[1]("second");
  assert.equal(await first, undefined);
  assert.deepEqual(await second, { type: "done", value: "second" });
});

void test("an answer that comes after the run was replaced is dropped even when the request ignores its signal", async () => {
  const run = new LatestRun<string>();
  let late: (value: string) => void = () => undefined;
  const slow = run.run(() => new Promise<string>((resolve) => (late = resolve)));
  const next = run.run(() => Promise.resolve("new"));
  late("old");
  assert.equal(await slow, undefined);
  assert.deepEqual(await next, { type: "done", value: "new" });
});

void test("an abort the run did not ask for is a failure", async () => {
  const run = new LatestRun<string>();
  const error = abortError();
  assert.deepEqual(await run.run(() => Promise.reject(error)), { type: "failed", error });
});

void test("a failure of the run is told, and one of a run given up on is not", async () => {
  const run = new LatestRun<string>();
  const error = new Error("invalid pattern");
  assert.deepEqual(await run.run(() => Promise.reject(error)), { type: "failed", error });
  const { ask, signals } = pending();
  const gone = run.run(ask);
  run.cancel();
  assert.equal(signals[0].aborted, true);
  assert.equal(await gone, undefined);
});
