import assert from "node:assert/strict";
import { beforeEach, mock, test } from "node:test";
import { abortError } from "../../../lib/abort.ts";
import { GotoLink, LINK_MS } from "./gotoLink.ts";

interface Target {
  key: string;
}
const foo: Target = { key: "foo" };
const bar: Target = { key: "bar" };

const answered = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

beforeEach(() => {
  mock.timers.reset();
  mock.timers.enable({ apis: ["setTimeout"] });
});

/** A host whose definitions exist for the keys in `defined`; its questions are answered at once unless `hold` is set. */
function setup(defined: string[], hold = false) {
  const asked: string[] = [];
  const aborted: string[] = [];
  const shown: (string | undefined)[] = [];
  const release: (() => void)[] = [];
  const link = new GotoLink<Target>({
    hasDefinition: (t, signal) => {
      asked.push(t.key);
      return new Promise((resolve, reject) => {
        signal.addEventListener("abort", () => {
          aborted.push(t.key);
          reject(abortError());
        });
        const answer = () => resolve(defined.includes(t.key));
        if (hold) release.push(answer);
        else answer();
      });
    },
    show: (t) => shown.push(t?.key),
  });
  return { link, asked, aborted, shown, release };
}

void test("a symbol with a definition becomes a link once the pointer has rested on it", async () => {
  const { link, shown, asked } = setup(["foo"]);
  link.update(foo, true);
  mock.timers.tick(LINK_MS - 1);
  assert.deepEqual(asked, []);
  mock.timers.tick(1);
  await answered();
  assert.deepEqual(shown, ["foo"]);
});

void test("without the modifier nothing is asked or shown", async () => {
  const { link, shown, asked } = setup(["foo"]);
  link.update(foo, false);
  mock.timers.tick(LINK_MS * 10);
  await answered();
  assert.deepEqual([asked, shown], [[], []]);
});

void test("a symbol without a definition is asked once while the pointer stays on it", async () => {
  const { link, shown, asked } = setup([]);
  for (let i = 0; i < 10; i++) {
    link.update(foo, true);
    mock.timers.tick(LINK_MS);
    await answered();
  }
  assert.deepEqual([asked, shown], [["foo"], []]);
});

void test("moving to another symbol cancels the question and takes the link away at once", async () => {
  const { link, shown, aborted, release } = setup(["foo", "bar"], true);
  link.update(foo, true);
  mock.timers.tick(LINK_MS);
  link.update(bar, true);
  assert.deepEqual(aborted, ["foo"]);
  mock.timers.tick(LINK_MS);
  release.forEach((answer) => answer());
  await answered();
  assert.deepEqual(shown, ["bar"]);
  link.update(foo, true);
  assert.deepEqual(shown, ["bar", undefined]);
});

void test("moving on before the pointer has rested asks nothing", async () => {
  const { link, asked } = setup(["foo", "bar"]);
  link.update(foo, true);
  mock.timers.tick(LINK_MS - 1);
  link.update(bar, true);
  mock.timers.tick(LINK_MS);
  await answered();
  assert.deepEqual(asked, ["bar"]);
});

void test("letting go of the modifier removes the link, and an answer still on its way shows none", async () => {
  const { link, shown, release } = setup(["foo"], true);
  link.update(foo, true);
  mock.timers.tick(LINK_MS);
  link.update(foo, false);
  release.forEach((answer) => answer());
  await answered();
  assert.deepEqual(shown, []);

  const second = setup(["foo"]);
  second.link.update(foo, true);
  mock.timers.tick(LINK_MS);
  await answered();
  second.link.update(foo, false);
  assert.deepEqual(second.shown, ["foo", undefined]);
});

void test("a question that fails leaves no link", async () => {
  const shown: unknown[] = [];
  const link = new GotoLink<Target>({
    hasDefinition: () => Promise.reject(new Error("server gone")),
    show: (t) => shown.push(t),
  });
  link.update(foo, true);
  mock.timers.tick(LINK_MS);
  await answered();
  assert.deepEqual(shown, []);
});
