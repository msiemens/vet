import assert from "node:assert/strict";
import { test } from "node:test";
import { refetchesWhole } from "./wholeFetch.ts";

void test("an event at the held revision reads nothing: a mark is written into the view", () => {
  assert.equal(refetchesWhole(7, 7), false);
});

void test("a settle at a newer revision reads the whole again", () => {
  assert.equal(refetchesWhole(8, 7), true);
});

void test("an event that names no revision moves the changeset: a target change or a resync reads again", () => {
  assert.equal(refetchesWhole(undefined, 7), true);
});

void test("nothing held yet is read whatever the event says", () => {
  assert.equal(refetchesWhole(7, undefined), true);
  assert.equal(refetchesWhole(undefined, undefined), true);
});

void test("an event at an older revision says nothing of the changeset held", () => {
  assert.equal(refetchesWhole(6, 7), false);
});
