import assert from "node:assert/strict";
import { test } from "node:test";
import { type ViewPoint, viewChange } from "./viewChange.ts";

const at = (revision: number, rest: Omit<ViewPoint, "revision"> = {}): ViewPoint => ({ revision, ...rest });

void test("the same view at the same revision is no change", () => {
  assert.equal(viewChange(at(3), at(3)), "none");
  assert.equal(viewChange(at(3, { range: { from: 1, to: 2 } }), at(3, { range: { from: 1, to: 2 } })), "none");
});

void test("a newer revision of the same view keeps the place", () => {
  assert.equal(viewChange(at(3), at(4)), "revision");
  assert.equal(viewChange(at(3, { commit: 1 }), at(4, { commit: 1 })), "revision");
});

void test("a range appearing or going starts over", () => {
  assert.equal(viewChange(at(3), at(3, { range: { from: 0, to: 2 } })), "start");
  assert.equal(viewChange(at(3, { range: { from: 0, to: 2 } }), at(3)), "start");
});

void test("a range appearing or going starts over even with a newer revision", () => {
  assert.equal(viewChange(at(3), at(4, { range: { from: 0, to: 2 } })), "start");
  assert.equal(viewChange(at(3, { range: { from: 0, to: 2 } }), at(4)), "start");
});

void test("a range from another version starts over", () => {
  assert.equal(viewChange(at(3, { range: { from: 0, to: 2 } }), at(3, { range: { from: 1, to: 2 } })), "start");
  assert.equal(viewChange(at(3, { range: { from: 0, to: 2 } }), at(4, { range: { from: 1, to: 3 } })), "start");
});

void test("a range to another version at the same revision starts over", () => {
  assert.equal(viewChange(at(3, { range: { from: 0, to: 2 } }), at(3, { range: { from: 0, to: 3 } })), "start");
});

void test("a range whose end moved with a new revision is the same range", () => {
  assert.equal(viewChange(at(3, { range: { from: 1, to: 2 } }), at(4, { range: { from: 1, to: 3 } })), "revision");
});

void test("another commit, or a commit appearing or going, starts over", () => {
  assert.equal(viewChange(at(3, { commit: 0 }), at(3, { commit: 1 })), "start");
  assert.equal(viewChange(at(3), at(3, { commit: 0 })), "start");
  assert.equal(viewChange(at(3, { commit: 0 }), at(3)), "start");
  assert.equal(viewChange(at(3, { commit: 0 }), at(4, { commit: 1 })), "start");
});

void test("the filter appearing or going starts over", () => {
  const filtered = new Set(["a.txt"]);
  assert.equal(viewChange(at(3), at(3, { filtered })), "start");
  assert.equal(viewChange(at(3, { filtered }), at(3)), "start");
});

void test("the filter is told apart by identity", () => {
  const filtered = new Set(["a.txt"]);
  assert.equal(viewChange(at(3, { filtered }), at(3, { filtered })), "none");
  assert.equal(viewChange(at(3, { filtered }), at(4, { filtered })), "revision");
  assert.equal(viewChange(at(3, { filtered }), at(3, { filtered: new Set(["a.txt"]) })), "start");
});

void test("a first view is a rebuild, and starts over when it is a part of the changeset", () => {
  assert.equal(viewChange(undefined, at(3)), "revision");
  assert.equal(viewChange(undefined, at(3, { range: { from: 0, to: 1 } })), "start");
  assert.equal(viewChange(undefined, at(3, { commit: 0 })), "start");
  assert.equal(viewChange(undefined, at(3, { filtered: new Set() })), "start");
});
