import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";
import { onMinute } from "./minute.ts";

afterEach(() => mock.timers.reset());

void test("listeners hear every minute on the wall clock and nothing after they stop listening", () => {
  mock.timers.enable({ apis: ["setTimeout", "Date"], now: Date.UTC(2026, 9, 14, 12, 0, 40) });
  const heard: number[] = [];
  const stop = onMinute(() => heard.push(Date.now()));

  mock.timers.tick(19_000);
  assert.deepEqual(heard, [], "not before the minute is over");
  mock.timers.tick(1_000);
  mock.timers.tick(60_000);
  assert.deepEqual(heard, [Date.UTC(2026, 9, 14, 12, 1, 0), Date.UTC(2026, 9, 14, 12, 2, 0)]);

  stop();
  mock.timers.tick(180_000);
  assert.equal(heard.length, 2);
});

void test("a listener that joins later shares the running timer and does not hear the minutes before", () => {
  mock.timers.enable({ apis: ["setTimeout", "Date"], now: Date.UTC(2026, 9, 14, 12, 0, 0) });
  const first: number[] = [];
  const second: number[] = [];
  const stopFirst = onMinute(() => first.push(Date.now()));
  mock.timers.tick(60_000);
  const stopSecond = onMinute(() => second.push(Date.now()));
  mock.timers.tick(60_000);
  stopFirst();
  mock.timers.tick(60_000);
  stopSecond();
  assert.equal(first.length, 2);
  assert.equal(second.length, 2);
});
