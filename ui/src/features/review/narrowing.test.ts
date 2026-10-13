import assert from "node:assert/strict";
import { test } from "node:test";
import type { ReviewFilter } from "../../generated/ReviewFilter.ts";
import { narrowingLabel } from "./narrowing.ts";

const project: ReviewFilter = {
  roots: ["src"],
  include: [],
  exclude: ["**/*.snap"],
  hideBinary: true,
  reviewGenerated: false,
};

void test("a narrowing to a folder names the folder", () => {
  assert.equal(narrowingLabel(project, { ...project, roots: ["src/api"] }), "Only src/api");
});

void test("a narrowing that lifts a rule names the files the rule left out", () => {
  assert.equal(narrowingLabel(project, { ...project, reviewGenerated: true }), "Only generated files");
  assert.equal(narrowingLabel(project, { ...project, roots: [] }), "Only files outside the roots");
  assert.equal(narrowingLabel(project, { ...project, exclude: [] }), "Only files matching **/*.snap");
  assert.equal(narrowingLabel(project, { ...project, hideBinary: false }), "Only binary files");
});
