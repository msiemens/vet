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

void test("a narrowing that brings back what a rule left out says it is in addition", () => {
  assert.equal(narrowingLabel(project, { ...project, reviewGenerated: true }), "Also generated");
  assert.equal(narrowingLabel(project, { ...project, roots: [] }), "Also outside roots");
  assert.equal(narrowingLabel(project, { ...project, exclude: [] }), "Also matching **/*.snap");
  assert.equal(narrowingLabel(project, { ...project, hideBinary: false }), "Also binary");
});

void test("a narrowing to a folder that also brings back a rule's files says both", () => {
  assert.equal(
    narrowingLabel(project, { ...project, roots: ["src/api"], reviewGenerated: true }),
    "Only src/api + generated",
  );
});
