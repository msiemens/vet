import assert from "node:assert/strict";
import { test } from "node:test";
import { ScreenReport } from "./screenReport.ts";

function setup() {
  const told: boolean[] = [];
  return { told, report: new ScreenReport((shown) => told.push(shown)) };
}

void test("a window that never shows the review screen tells the backend nothing", () => {
  const { told, report } = setup();
  report.update(false, true);
  report.update(false, false);
  assert.deepEqual(told, []);
});

void test("showing the review screen is told once the changeset asked for is on view", () => {
  const { told, report } = setup();
  report.update(true, false);
  assert.deepEqual(told, []);
  report.update(true, true);
  report.update(true, true);
  assert.deepEqual(told, [true]);
});

void test("leaving the review screen is told at once, even while a changeset is on its way", () => {
  const { told, report } = setup();
  report.update(true, true);
  report.update(false, false);
  assert.deepEqual(told, [true, false]);
});

void test("a backend that lost what the window showed is told again", () => {
  const { told, report } = setup();
  report.update(true, true);
  report.forget();
  report.update(true, true);
  assert.deepEqual(told, [true, true]);
});

void test("a new connection to a window on the queue needs no telling", () => {
  const { told, report } = setup();
  report.update(true, true);
  report.update(false, true);
  report.forget();
  report.update(false, true);
  assert.deepEqual(told, [true, false]);
});
