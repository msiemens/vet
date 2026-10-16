import assert from "node:assert/strict";
import { test } from "node:test";
import type { Thread } from "../generated/Thread.ts";
import { type ThreadCell, cellFor, cellListed, replaces } from "./threadCells.ts";

const thread = (id: number) => ({ id }) as Thread;
const cell = (revision: number, key: string, ...ids: number[]): ThreadCell => ({
  revision,
  key,
  items: ids.map(thread),
});

const whole = cell(5, "", 1, 2, 3);
const range = cell(5, "1-2", 3);

void test("the changeset and a commit are served by the whole cell, a range by its own", () => {
  const cells = { whole, range };
  assert.equal(cellFor(cells, undefined), whole);
  assert.equal(cellFor(cells, { from: 1, to: 2 }), range);
});

void test("a range other than the one the cell was read in has no cell", () => {
  assert.equal(cellFor({ whole, range }, { from: 1, to: 3 }), undefined);
  assert.equal(cellFor({ whole }, { from: 1, to: 2 }), undefined);
});

void test("a list beside a range shows the changeset's threads until the range's are read", () => {
  assert.equal(cellListed({ whole, range }, { from: 1, to: 2 }), range);
  assert.equal(cellListed({ whole }, { from: 1, to: 2 }), whole);
  assert.equal(cellListed({ whole, range }, undefined), whole);
});

void test("an answer for an older revision does not replace a newer one", () => {
  assert.equal(replaces(cell(6, ""), cell(5, "")), false);
  assert.equal(replaces(cell(6, ""), cell(6, "")), true);
  assert.equal(replaces(cell(6, ""), cell(7, "")), true);
  assert.equal(replaces(undefined, cell(1, "")), true);
});
