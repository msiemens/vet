import assert from "node:assert/strict";
import { test } from "node:test";
import type { NonTextItem } from "../../../generated/NonTextItem.ts";
import { cardContent } from "./cardContent.ts";

const entry = { kind: "file", mode: 0o100644, target: null, size: 2048 } as const;
const item = (kind: NonTextItem["kind"], before: NonTextItem["before"], after: NonTextItem["after"]): NonTextItem => ({
  kind,
  before,
  after,
  hasPreview: false,
  modeChange: null,
});

void test("a change shows the before and after rows, a missing side as none", () => {
  const { title, rows } = cardContent(item("binary", entry, null), "a.bin", null);
  assert.equal(title, "Binary file");
  assert.deepEqual(rows, [
    { type: "change", sign: "−", text: "2.0 KiB" },
    { type: "change", sign: "+", text: "(none)" },
  ]);
});

void test("a rename shows the old and the new path", () => {
  const { rows } = cardContent(item("rename", entry, entry), "new.rs", "old.rs");
  assert.deepEqual(
    rows.map((r) => r.text),
    ["old.rs", "new.rs"],
  );
});

void test("content that is not available is one note", () => {
  const { title, rows } = cardContent(item("contentUnavailable", null, null), "a.bin", null);
  assert.equal(title, "Content unavailable");
  assert.deepEqual(rows, [{ type: "note", text: "The content of this file is not available." }]);
});
