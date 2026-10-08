import assert from "node:assert/strict";
import { test } from "node:test";
import { abortError } from "../lib/abort.ts";
import { LogBudget, type Report, reportFailedRequest, reportUncaught } from "./errorLog.ts";

void test("a burst of entries is cut off at the budget and the next window starts afresh", () => {
  const budget = new LogBudget(3, 1000);
  const admitted = [0, 1, 2, 3, 4, 999].map((at) => budget.admit(at));
  assert.deepEqual(admitted, [true, true, true, false, false, false]);
  assert.equal(budget.admit(1000), true);
});

void test("uncaught errors and unhandled rejections are reported, and a given-up request is not", () => {
  const reports: [string, string, string][] = [];
  const report: Report = (level, message, context) => reports.push([level, message, context]);
  const target = new EventTarget();
  const stop = reportUncaught(report, target);

  target.dispatchEvent(Object.assign(new Event("error"), { message: "boom", error: undefined }));
  target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: new Error("lost") }));
  target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: abortError() }));
  assert.deepEqual(
    reports.map(([level, message, context]) => [level, message.split("\n")[0], context]),
    [
      ["error", "boom", "uncaught error"],
      ["error", "Error: lost", "unhandled rejection"],
    ],
  );

  stop();
  target.dispatchEvent(Object.assign(new Event("error"), { message: "after", error: undefined }));
  assert.equal(reports.length, 2);
});

void test("an error event without an error object is reported by its message and position, a ResizeObserver loop as a warning", () => {
  const reports: [string, string, string][] = [];
  const report: Report = (level, message, context) => reports.push([level, message, context]);
  const target = new EventTarget();
  reportUncaught(report, target);

  target.dispatchEvent(
    Object.assign(new Event("error"), {
      message: "ResizeObserver loop completed with undelivered notifications.",
      error: null,
      filename: "",
    }),
  );
  target.dispatchEvent(
    Object.assign(new Event("error"), {
      message: "Script error.",
      error: null,
      filename: "http://localhost/src/App.svelte",
      lineno: 12,
      colno: 3,
    }),
  );
  target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: null }));
  assert.deepEqual(reports, [
    ["warn", "ResizeObserver loop completed with undelivered notifications.", "uncaught error"],
    ["error", "Script error. at http://localhost/src/App.svelte:12:3", "uncaught error"],
    ["error", "a promise rejected with null", "unhandled rejection"],
  ]);
});

void test("a failed request is reported by its kind alone, and a cancelled one is not", () => {
  const reports: [string, string, string][] = [];
  const report: Report = (level, message, context) => reports.push([level, message, context]);
  reportFailedRequest(report, "hover", new Error("no such file"));
  reportFailedRequest(report, "hover", abortError());
  assert.deepEqual(reports, [["warn", "no such file", "request hover"]]);
});
