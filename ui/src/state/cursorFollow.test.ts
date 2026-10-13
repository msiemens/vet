import assert from "node:assert/strict";
import { beforeEach, mock, test } from "node:test";
import { CursorFollow } from "./cursorFollow.ts";

const MS = 100;

interface Asked {
  line: number;
  signal: AbortSignal;
  answer: (symbol: string) => void;
  fail: (error: unknown) => void;
}

/** A follower whose backend answers when the test says so. */
function setup() {
  const asked: Asked[] = [];
  const applied: string[] = [];
  const failed: unknown[] = [];
  const follow = new CursorFollow<number, string>({
    ms: MS,
    key: String,
    ask: (line, signal) =>
      new Promise((resolve, reject) => asked.push({ line, signal, answer: resolve, fail: reject })),
    apply: (symbol) => applied.push(symbol),
    fail: (error) => failed.push(error),
  });
  return { follow, asked, applied, failed };
}

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => {
  mock.timers.reset();
  mock.timers.enable({ apis: ["setTimeout"] });
});

void test("a cursor that keeps moving is asked about once it rests, at its last place", () => {
  const { follow, asked } = setup();
  for (const line of [1, 2, 3, 4]) {
    follow.move(line);
    mock.timers.tick(MS - 1);
  }
  assert.equal(asked.length, 0);
  mock.timers.tick(1);
  assert.deepEqual(
    asked.map((a) => a.line),
    [4],
  );
});

void test("the request for a place the cursor has left is cancelled and its answer dropped", async () => {
  const { follow, asked, applied } = setup();
  follow.move(1);
  mock.timers.tick(MS);
  follow.move(2);
  mock.timers.tick(MS);
  assert.equal(asked[0].signal.aborted, true);
  asked[0].answer("first");
  asked[1].answer("second");
  await settle();
  assert.deepEqual(applied, ["second"]);
});

void test("the place being asked about is not asked about again", () => {
  const { follow, asked } = setup();
  follow.move(7);
  mock.timers.tick(MS);
  follow.move(7);
  mock.timers.tick(MS);
  assert.equal(asked.length, 1);
});

void test("a cursor back at the place being asked about leaves that request alone", () => {
  const { follow, asked } = setup();
  follow.move(1);
  mock.timers.tick(MS);
  follow.move(2);
  follow.move(1);
  mock.timers.tick(MS);
  assert.equal(asked.length, 1);
  assert.equal(asked[0].signal.aborted, false);
});

void test("a failed request is reported and its place asked about again", async () => {
  const { follow, asked, failed } = setup();
  follow.move(1);
  mock.timers.tick(MS);
  asked[0].fail(new Error("no server"));
  await settle();
  assert.equal(failed.length, 1);
  follow.move(1);
  mock.timers.tick(MS);
  assert.equal(asked.length, 2);
});

void test("an ask that throws is a failure, and its place is asked about again", async () => {
  const failed: unknown[] = [];
  let calls = 0;
  const follow = new CursorFollow<number, string>({
    ms: MS,
    key: String,
    ask: () => {
      calls++;
      throw new Error("no repository is open");
    },
    apply: () => undefined,
    fail: (error) => failed.push(error),
  });
  follow.move(1);
  mock.timers.tick(MS);
  await settle();
  assert.equal(failed.length, 1);
  follow.move(1);
  mock.timers.tick(MS);
  assert.equal(calls, 2);
});

void test("a cancelled request is no failure", async () => {
  const { follow, asked, failed } = setup();
  follow.move(1);
  mock.timers.tick(MS);
  asked[0].fail(new DOMException("aborted", "AbortError"));
  await settle();
  assert.deepEqual(failed, []);
});

void test("after a reset the same place is asked about again and what was underway is cancelled", () => {
  const { follow, asked } = setup();
  follow.move(1);
  mock.timers.tick(MS);
  follow.reset();
  assert.equal(asked[0].signal.aborted, true);
  follow.move(1);
  mock.timers.tick(MS);
  assert.equal(asked.length, 2);
});
