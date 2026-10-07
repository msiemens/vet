import assert from "node:assert/strict";
import { test } from "node:test";
import { toggleTool } from "./toggleTool.ts";

void test("the selected tool hides and shows, another is selected and shown", () => {
  assert.deepEqual(toggleTool({ tool: "files", open: true }, "files"), { tool: "files", open: false });
  assert.deepEqual(toggleTool({ tool: "files", open: false }, "files"), { tool: "files", open: true });
  assert.deepEqual(toggleTool({ tool: "files", open: false }, "comments"), { tool: "comments", open: true });
  assert.deepEqual(toggleTool({ tool: "files", open: true }, "comments"), { tool: "comments", open: true });
});
