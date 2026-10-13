import assert from "node:assert/strict";
import { test } from "node:test";
import { DefinitionProbe } from "./definitionProbe.ts";

void test("a position is asked once, and its answer is kept", async () => {
  const asked: string[] = [];
  const probe = new DefinitionProbe((key) => {
    asked.push(key);
    return Promise.resolve(key === "a");
  });
  assert.equal(await probe.has("a"), true);
  assert.equal(await probe.has("a"), true);
  assert.equal(await probe.has("b"), false);
  assert.equal(await probe.has("b"), false);
  assert.deepEqual(asked, ["a", "b"]);
});

void test("asking again while the question is underway joins it", async () => {
  let asked = 0;
  let answer: (found: boolean) => void = () => undefined;
  const probe = new DefinitionProbe(() => {
    asked++;
    return new Promise((resolve) => (answer = resolve));
  });
  const first = probe.has("a");
  const second = probe.has("a");
  answer(true);
  assert.deepEqual([await first, await second, asked], [true, true, 1]);
});

void test("a failed question is asked again", async () => {
  let asked = 0;
  const probe = new DefinitionProbe(() => {
    asked++;
    return asked === 1 ? Promise.reject(new Error("server gone")) : Promise.resolve(true);
  });
  await assert.rejects(probe.has("a"));
  assert.equal(await probe.has("a"), true);
});

void test("clearing forgets the answers, and an answer to an earlier question is not kept", async () => {
  let asked = 0;
  let answer: (found: boolean) => void = () => undefined;
  const probe = new DefinitionProbe(() => {
    asked++;
    return new Promise((resolve) => (answer = resolve));
  });
  const stale = probe.has("a");
  probe.clear();
  answer(false);
  await stale;
  const fresh = probe.has("a");
  answer(true);
  assert.equal(await fresh, true);
  assert.equal(asked, 2);
  probe.clear();
  const again = probe.has("a");
  answer(true);
  await again;
  assert.equal(asked, 3);
});
