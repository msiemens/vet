import assert from "node:assert/strict";
import { test } from "node:test";
import { type SavedView, restoredTabs } from "./changesetViews.ts";

const saved = (open: string[], active: string): SavedView => ({
  open: open as SavedView["open"],
  active: active as SavedView["active"],
  scroll: new Map(),
});

const exists =
  (...paths: string[]) =>
  (tab: string) =>
    paths.some((p) => tab === `file:${p}`);

void test("a changeset seen for the first time has the Changes tab alone", () => {
  assert.deepEqual(restoredTabs(undefined, exists("a.rs")), { open: ["changes"], active: "changes" });
});

void test("a changeset comes back with its tabs in order and the active one", () => {
  const view = saved(["changes", "file:b.rs", "file:a.rs"], "file:a.rs");
  assert.deepEqual(restoredTabs(view, exists("a.rs", "b.rs")), {
    open: ["changes", "file:b.rs", "file:a.rs"],
    active: "file:a.rs",
  });
});

void test("a tab whose file the changeset no longer has is not restored, and focus falls back to Changes", () => {
  const view = saved(["changes", "file:gone.rs", "file:a.rs"], "file:gone.rs");
  assert.deepEqual(restoredTabs(view, exists("a.rs")), { open: ["changes", "file:a.rs"], active: "changes" });
});
