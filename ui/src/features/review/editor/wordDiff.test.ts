import assert from "node:assert/strict";
import { test } from "node:test";
import { markDifferences } from "./wordDiff.ts";

const changed = (text: string, other: string) =>
  markDifferences(text, other)
    .filter((p) => p.changed)
    .map((p) => p.text);

void test("only the words the other text lacks are marked, and the pieces join back to the text", () => {
  const mine = "Please add a TTL to the cache.";
  const theirs = "Please add a timeout to the cache.";
  assert.deepEqual(changed(mine, theirs), ["TTL"]);
  assert.deepEqual(changed(theirs, mine), ["timeout"]);
  assert.equal(
    markDifferences(mine, theirs)
      .map((p) => p.text)
      .join(""),
    mine,
  );
});

void test("texts that differ only in the white space between words have nothing marked", () => {
  assert.deepEqual(changed("a  b", "a b"), []);
});

void test("a text with nothing in common is marked word by word", () => {
  assert.deepEqual(changed("one two", "three"), ["one", "two"]);
});
