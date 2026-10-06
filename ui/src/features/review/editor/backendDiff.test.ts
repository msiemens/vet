import assert from "node:assert/strict";
import { test } from "node:test";
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

void test("adjacent changes are kept as given", () => {
  const changes = [change(0, 3, 0, 5), change(6, 7, 8, 10)];
  const diff = backendDiff("a\nb\nc\nd\n", "a2\nb2\nc\nd2\n", changes);
  assert.deepEqual(plain(diff("a\nb\nc\nd\n", "a2\nb2\nc\nd2\n")), [
    [0, 3, 0, 5],
    [6, 7, 8, 10],
  ]);
});
