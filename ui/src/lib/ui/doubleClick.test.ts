import assert from "node:assert/strict";
import { test } from "node:test";
import { ClickGate } from "./doubleClick.ts";

function gate() {
  const calls: string[] = [];
  return {
    calls,
    gate: new ClickGate<string>(
      (s) => calls.push(`single ${s}`),
      (s) => calls.push(`double ${s}`),
      10,
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
