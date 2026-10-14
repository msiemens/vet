import assert from "node:assert/strict";
import { test } from "node:test";
import { loadOnce } from "./loadOnce.ts";

void test("a load that succeeded is shared, not started again", async () => {
  const slot: { loading?: Promise<number> } = {};
  let started = 0;
  const start = () => Promise.resolve(++started);
  assert.equal(await loadOnce(slot, start, assert.fail), 1);
  assert.equal(await loadOnce(slot, start, assert.fail), 1);
});

void test("a load that failed is reported once and tried again by the next call", async () => {
  const slot: { loading?: Promise<string> } = {};
  const reported: unknown[] = [];
  const first = loadOnce(
    slot,
    () => Promise.reject(new Error("gone")),
    (e) => reported.push(e),
  );
  const alongside = loadOnce(
    slot,
    () => Promise.resolve("never"),
    (e) => reported.push(e),
  );
  await assert.rejects(first, /gone/);
  await assert.rejects(alongside, /gone/);
  assert.equal(reported.length, 1);

  assert.equal(await loadOnce(slot, () => Promise.resolve("back"), assert.fail), "back");
});
