import assert from "node:assert/strict";
import { test } from "node:test";
import { ClickGate, FALLBACK_DOUBLE_CLICK_MS, doubleClickMs, setDoubleClickMs } from "./doubleClick.ts";

function gate(ms = () => 10) {
  const calls: string[] = [];
  return {
    calls,
    gate: new ClickGate<string>(
      (s) => calls.push(`single ${s}`),
      (s) => calls.push(`double ${s}`),
      ms,
    ),
  };
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

void test("a lone click acts once the interval has passed", async () => {
  const { calls, gate: g } = gate();
  g.click({ detail: 1 }, "a");
  assert.deepEqual(calls, []);
  await wait(30);
  assert.deepEqual(calls, ["single a"]);
});

void test("a double click only runs the double action, never the single one", async () => {
  const { calls, gate: g } = gate();
  g.click({ detail: 1 }, "a");
  g.click({ detail: 2 }, "a");
  await wait(30);
  assert.deepEqual(calls, ["double a"]);
});

void test("a click from the keyboard acts at once", () => {
  const { calls, gate: g } = gate();
  g.click({ detail: 0 }, "a");
  assert.deepEqual(calls, ["single a"]);
});

void test("a click on another subject replaces the one waiting", async () => {
  const { calls, gate: g } = gate();
  g.click({ detail: 1 }, "a");
  g.click({ detail: 1 }, "b");
  await wait(30);
  assert.deepEqual(calls, ["single b"]);
});

void test("a cancelled click never acts", async () => {
  const { calls, gate: g } = gate();
  g.click({ detail: 1 }, "a");
  g.cancel();
  await wait(30);
  assert.deepEqual(calls, []);
});

void test("a click waits for the double-click time as it is when the click comes", async () => {
  let ms = 60;
  const { calls, gate: g } = gate(() => ms);
  g.click({ detail: 1 }, "a");
  await wait(30);
  assert.deepEqual(calls, [], "the first click is still waiting for a second");
  await wait(60);
  assert.deepEqual(calls, ["single a"]);
  ms = 5;
  g.click({ detail: 1 }, "b");
  await wait(25);
  assert.deepEqual(calls, ["single a", "single b"]);
});

void test("Enter after a click that waits runs the double action alone, and Space the single one at once", async () => {
  const { calls, gate: g } = gate();
  g.click({ detail: 1 }, "a");
  g.press("double", "a");
  await wait(30);
  assert.deepEqual(calls, ["double a"]);
  g.click({ detail: 1 }, "b");
  g.press("single", "b");
  await wait(30);
  assert.deepEqual(calls, ["double a", "single b"]);
});

void test("the double-click time is the system's once the host has told it, else 500 ms", () => {
  assert.equal(doubleClickMs(), FALLBACK_DOUBLE_CLICK_MS);
  setDoubleClickMs(0);
  setDoubleClickMs(Number.NaN);
  assert.equal(doubleClickMs(), FALLBACK_DOUBLE_CLICK_MS);
  setDoubleClickMs(320);
  assert.equal(doubleClickMs(), 320);
  setDoubleClickMs(FALLBACK_DOUBLE_CLICK_MS);
});
