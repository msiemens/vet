import assert from "node:assert/strict";
import { test } from "node:test";
import { type SavedView, type Storage, ViewMemory, restoredTabs } from "./changesetViews.ts";

const saved = (open: string[], active: string, modes: Record<string, string> = {}): SavedView => ({
  open: open as SavedView["open"],
  active: active as SavedView["active"],
  modes: new Map(Object.entries(modes)) as SavedView["modes"],
  panes: new Map(),
});

const changed =
  (...paths: string[]) =>
  (path: string) =>
    paths.includes(path);

const noLogs = () => false;

/** A storage that outlives the memories built on it, as the browser's does a restart. */
function fakeStorage(): Storage & { items: Map<string, string> } {
  const items = new Map<string, string>();
  return {
    items,
    read: (key) => items.get(key),
    write: (key, value) => {
      if (value === undefined) items.delete(key);
      else items.set(key, value);
    },
  };
}

void test("a changeset seen for the first time has the Changes tab alone, in the diff mode last chosen", () => {
  const { open, active, modes } = restoredTabs(undefined, changed("a.rs"), "split", noLogs);
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
  const { open, active, modes } = restoredTabs(view, changed("a.rs", "b.rs"), "split", noLogs);
  assert.deepEqual(open, ["changes", "file:b.rs", "file:a.rs"]);
  assert.equal(active, "file:a.rs");
  assert.deepEqual([...modes], [...view.modes]);
});

void test("a diff of a file the changeset no longer changes is not restored, and focus falls back to Changes", () => {
  const view = saved(["changes", "file:gone.rs", "file:a.rs"], "file:gone.rs", {
    "file:gone.rs": "unified",
    "file:a.rs": "unified",
  });
  const { open, active } = restoredTabs(view, changed("a.rs"), "unified", noLogs);
  assert.deepEqual(open, ["changes", "file:a.rs"]);
  assert.equal(active, "changes");
});

void test("a file's text is restored whether or not the changeset changes the file", () => {
  const view = saved(["changes", "file:lib.rs"], "file:lib.rs", { "file:lib.rs": "file" });
  const { open, active } = restoredTabs(view, changed(), "unified", noLogs);
  assert.deepEqual(open, ["changes", "file:lib.rs"]);
  assert.equal(active, "file:lib.rs");
});

void test("a log tab is restored only while its log is held", () => {
  const view = saved(["changes", "log:7", "log:8"], "log:7");
  const { open } = restoredTabs(view, changed(), "unified", (tab) => tab === "log:8");
  assert.deepEqual(open, ["changes", "log:8"]);
});

const layout = (): SavedView => ({
  open: ["changes", "file:a.rs", "file:b.rs"],
  active: "file:b.rs",
  modes: new Map([
    ["changes", "split"],
    ["file:a.rs", "unified"],
    ["file:b.rs", "file"],
  ]),
  panes: new Map([
    ["file:a.rs", { diff: { top: 12, caret: 14 } }],
    ["file:b.rs", { diff: { top: 3, caret: 3 }, text: { top: 40, caret: 44 } }],
  ]),
  changes: { path: "a.rs", offset: -8.5 },
  anchor: { path: "a.rs", line: 20 },
});

void test("tabs, modes, positions and the hunk survive a restart", () => {
  const storage = fakeStorage();
  new ViewMemory("/repo", storage).set("pr:1", layout());
  const back = new ViewMemory("/repo", storage).get("pr:1");
  assert.deepEqual(back, layout());
});

void test("changesets of one repository keep their own views, and other repositories theirs", () => {
  const storage = fakeStorage();
  const memory = new ViewMemory("/repo", storage);
  memory.set("pr:1", layout());
  memory.set("pr:2", saved(["changes", "file:z.rs"], "changes", { "file:z.rs": "split" }));
  assert.deepEqual(memory.get("pr:1"), layout());
  assert.deepEqual(memory.get("pr:2")?.open, ["changes", "file:z.rs"]);
  assert.equal(new ViewMemory("/other", storage).get("pr:1"), undefined);
});

void test("a window that saves does not undo what another window saved meanwhile", () => {
  const storage = fakeStorage();
  const first = new ViewMemory("/repo", storage);
  const second = new ViewMemory("/repo", storage);
  first.set("pr:1", layout());
  second.set("pr:2", saved(["changes", "file:z.rs"], "changes", { "file:z.rs": "file" }));
  first.set("pr:3", saved(["changes"], "changes"));
  const back = new ViewMemory("/repo", storage);
  assert.deepEqual(back.get("pr:1"), layout());
  assert.deepEqual(back.get("pr:2")?.open, ["changes", "file:z.rs"]);
  assert.ok(back.get("pr:3"));
});

void test("log tabs stay in memory but are not stored", () => {
  const storage = fakeStorage();
  const memory = new ViewMemory("/repo", storage);
  memory.set("pr:1", saved(["changes", "log:9", "file:a.rs"], "log:9", { "file:a.rs": "unified" }));
  assert.deepEqual(memory.get("pr:1")?.open, ["changes", "log:9", "file:a.rs"]);
  const back = new ViewMemory("/repo", storage).get("pr:1");
  assert.deepEqual(back?.open, ["changes", "file:a.rs"]);
  assert.equal(back.active, "changes");
});

void test("only the most recently set changesets are kept", () => {
  const storage = fakeStorage();
  const memory = new ViewMemory("/repo", storage, 2);
  memory.set("a", saved(["changes"], "changes"));
  memory.set("b", saved(["changes"], "changes"));
  memory.set("a", saved(["changes", "file:x.rs"], "changes", { "file:x.rs": "file" }));
  memory.set("c", saved(["changes"], "changes"));
  const back = new ViewMemory("/repo", storage, 2);
  assert.equal(back.get("b"), undefined);
  assert.deepEqual(back.get("a")?.open, ["changes", "file:x.rs"]);
  assert.ok(back.get("c"));
});

void test("damaged storage gives no views, and damaged parts are left out of the rest", () => {
  const storage = fakeStorage();
  storage.items.set("vet.tabs:/repo", "{");
  assert.equal(new ViewMemory("/repo", storage).get("a"), undefined);
  storage.items.set(
    "vet.tabs:/repo",
    JSON.stringify([
      [
        "a",
        {
          open: ["file:x.rs", "bogus", 3],
          active: "bogus",
          modes: { "file:x.rs": "sideways", changes: "split" },
          panes: { "file:x.rs": { diff: { top: -1, caret: 2 }, text: { top: 1.5, caret: 0 } } },
          changes: { path: "x.rs" },
        },
      ],
      ["b", 7],
    ]),
  );
  const memory = new ViewMemory("/repo", storage);
  assert.equal(memory.get("b"), undefined);
  const a = memory.get("a");
  assert.deepEqual(a?.open, ["changes", "file:x.rs"]);
  assert.equal(a.active, "changes");
  assert.deepEqual([...a.modes], [["changes", "split"]]);
  assert.equal(a.panes.size, 0);
  assert.equal(a.changes, undefined);
});
