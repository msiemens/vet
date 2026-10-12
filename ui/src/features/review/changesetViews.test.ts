import assert from "node:assert/strict";
import { test } from "node:test";
import { type SavedView, restoredTabs } from "./changesetViews.ts";

const saved = (open: string[], active: string, modes: Record<string, string> = {}): SavedView => ({
  open: open as SavedView["open"],
  active: active as SavedView["active"],
  modes: new Map(Object.entries(modes)) as SavedView["modes"],
  scroll: new Map(),
});

const changed =
  (...paths: string[]) =>
  (path: string) =>
    paths.includes(path);

void test("a changeset seen for the first time has the Changes tab alone, in the diff mode last chosen", () => {
  const { open, active, modes } = restoredTabs(undefined, changed("a.rs"), "split");
  assert.deepEqual(
    { open, active, modes: [...modes] },
    { open: ["changes"], active: "changes", modes: [["changes", "split"]] },
  );
});

void test("a changeset comes back with its tabs in order, their modes and the active one", () => {
  const view = saved(["changes", "file:b.rs", "file:a.rs"], "file:a.rs", {
    changes: "unified",
    "file:b.rs": "split",
    "file:a.rs": "file",
  });
  const { open, active, modes } = restoredTabs(view, changed("a.rs", "b.rs"), "split");
  assert.deepEqual(open, ["changes", "file:b.rs", "file:a.rs"]);
  assert.equal(active, "file:a.rs");
  assert.deepEqual([...modes], [...view.modes]);
});

void test("a diff of a file the changeset no longer changes is not restored, and focus falls back to Changes", () => {
  const view = saved(["changes", "file:gone.rs", "file:a.rs"], "file:gone.rs", {
    "file:gone.rs": "unified",
    "file:a.rs": "unified",
  });
  const { open, active } = restoredTabs(view, changed("a.rs"), "unified");
  assert.deepEqual(open, ["changes", "file:a.rs"]);
  assert.equal(active, "changes");
});

void test("a file's text is restored whether or not the changeset changes the file", () => {
  const view = saved(["changes", "file:lib.rs"], "file:lib.rs", { "file:lib.rs": "file" });
  const { open, active } = restoredTabs(view, changed(), "unified");
  assert.deepEqual(open, ["changes", "file:lib.rs"]);
  assert.equal(active, "file:lib.rs");
});
