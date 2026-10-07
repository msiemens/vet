import assert from "node:assert/strict";
import { test } from "node:test";
import { commitToShow, neighbour } from "./commitLens.ts";

void test("the squash lens, and a changeset with one commit or none, show no commit", () => {
  assert.equal(commitToShow("squash", 5, 2), undefined);
  assert.equal(commitToShow("perCommit", 1, 0), undefined);
  assert.equal(commitToShow("perCommit", 0, 0), undefined);
});

void test("by commit shows the commit that was on view, kept inside the list", () => {
  assert.equal(commitToShow("perCommit", 5, 2), 2);
  assert.equal(commitToShow("perCommit", 3, 7), 2, "the list got shorter");
  assert.equal(commitToShow("perCommit", 3, -1), 0);
});

void test("stepping stops at the ends of the list", () => {
  assert.equal(neighbour(1, 1, 5), 2);
  assert.equal(neighbour(1, -1, 5), 0);
  assert.equal(neighbour(0, -1, 5), undefined);
  assert.equal(neighbour(4, 1, 5), undefined);
});
