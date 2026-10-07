import assert from "node:assert/strict";
import { test } from "node:test";
import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import type { TextChange } from "../../../generated/TextChange.ts";
import { backendDiff } from "./backendDiff.ts";

const change = (fromBase: number, toBase: number, fromTarget: number, toTarget: number): TextChange => ({
  fromBase,
  toBase,
  fromTarget,
  toTarget,
});
const plain = (changes: readonly { fromA: number; toA: number; fromB: number; toB: number }[]) =>
  changes.map((c) => [c.fromA, c.toA, c.fromB, c.toB]);

void test("the whole documents get the backend's changes", () => {
  const diff = backendDiff("a\nb\n", "a\nx\nb\n", [change(2, 2, 2, 4)]);
  assert.deepEqual(plain(diff("a\nb\n", "a\nx\nb\n")), [[2, 2, 2, 4]]);
});

void test("pure deletions and a file without a trailing newline", () => {
  const diff = backendDiff("a\nb", "a", [change(1, 3, 1, 1)]);
  assert.deepEqual(plain(diff("a\nb", "a")), [[1, 3, 1, 1]]);
});

void test("CRLF documents are matched as CodeMirror holds them", () => {
  const diff = backendDiff("a\r\nb\r\n", "a\r\nc\r\n", [change(2, 3, 2, 3)]);
  assert.deepEqual(plain(diff("a\nb\n", "a\nc\n")), [[2, 3, 2, 3]]);
});

void test("other texts are diffed", () => {
  const diff = backendDiff("a\nb\n", "a\nc\n", [change(2, 3, 2, 3)]);
  assert.deepEqual(plain(diff("b", "c")), [[0, 1, 0, 1]]);
});

// CodeMirror's chunks of the backend's changes, as [fromA, toA, fromB, toB].
function chunks(base: string, target: string, changes: TextChange[]) {
  const diff = backendDiff(base, target, changes);
  const built = Chunk.build(Text.of(base.split("\n")), Text.of(target.split("\n")), { override: diff });
  return built.map((c) => [c.fromA, c.toA, c.fromB, c.toB]);
}

void test("a deleted empty line before another one is a chunk of that line", () => {
  // Lines 1 of both texts are empty: a change there that begins at the line start is taken for a line break.
  assert.deepEqual(chunks("x\n\n\ny\n", "x\n\ny\n", [change(1, 3, 1, 2)]), [[2, 3, 2, 2]]);
});

void test("inserted lines equal to the line after them stay where they were inserted", () => {
  const base = "x\nfoo\nbaz\n";
  const target = "x\nfoo\nbar\nfoo\nbaz\n";
  assert.deepEqual(chunks(base, target, [change(1, 2, 1, 10)]), [[2, 2, 2, 10]]);
});

void test("adjacent changes are kept as given", () => {
  const changes = [change(0, 3, 0, 5), change(6, 7, 8, 10)];
  const diff = backendDiff("a\nb\nc\nd\n", "a2\nb2\nc\nd2\n", changes);
  assert.deepEqual(plain(diff("a\nb\nc\nd\n", "a2\nb2\nc\nd2\n")), [
    [0, 3, 0, 5],
    [6, 7, 8, 10],
  ]);
});
