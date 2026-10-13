import assert from "node:assert/strict";
import { test } from "node:test";
import { toggleBottom, toggleTool } from "./toggleTool.ts";

void test("the selected tool hides and shows, another is selected and shown", () => {
  assert.deepEqual(toggleTool({ tool: "changes", open: true }, "changes"), { tool: "changes", open: false });
  assert.deepEqual(toggleTool({ tool: "changes", open: false }, "changes"), { tool: "changes", open: true });
  assert.deepEqual(toggleTool({ tool: "changes", open: false }, "comments"), { tool: "comments", open: true });
  assert.deepEqual(toggleTool({ tool: "changes", open: true }, "comments"), { tool: "comments", open: true });
});

void test("the bottom tools exclude each other: the one shown hides, another takes its place", () => {
  assert.equal(toggleBottom(undefined, "problems"), "problems");
  assert.equal(toggleBottom("problems", "problems"), undefined);
  assert.equal(toggleBottom("timeline", "problems"), "problems");
  assert.equal(toggleBottom("problems", "timeline"), "timeline");
});
