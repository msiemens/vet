import assert from "node:assert/strict";
import { test } from "node:test";
import { parseToolWindow, toggleTool } from "./toggleTool.ts";

void test("a kept window is restored only if it names a tool of the rail", () => {
  const tools = ["changes", "comments"];
  assert.deepEqual(parseToolWindow({ tool: "comments", open: false }, tools), { tool: "comments", open: false });
  for (const kept of [null, "changes", { tool: "gone", open: true }, { tool: "changes" }, { tool: 1, open: true }]) {
    assert.equal(parseToolWindow(kept, tools), undefined, JSON.stringify(kept));
  }
});

void test("the selected tool hides and shows, another is selected and shown", () => {
  assert.deepEqual(toggleTool({ tool: "changes", open: true }, "changes"), { tool: "changes", open: false });
  assert.deepEqual(toggleTool({ tool: "changes", open: false }, "changes"), { tool: "changes", open: true });
  assert.deepEqual(toggleTool({ tool: "changes", open: false }, "comments"), { tool: "comments", open: true });
  assert.deepEqual(toggleTool({ tool: "changes", open: true }, "comments"), { tool: "comments", open: true });
});
