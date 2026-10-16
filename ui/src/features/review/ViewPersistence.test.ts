import assert from "node:assert/strict";
import { test } from "node:test";
import { type SavedView, type Storage, ViewMemory } from "./changesetViews.ts";
import { ViewPersistence } from "./ViewPersistence.ts";

/** A storage that outlives the persistence built on it, as the browser's does a restart. */
function fakeStorage(): Storage & { text: string | undefined } {
  const kept = {
    text: undefined as string | undefined,
    read: () => kept.text,
    write: (value: string | undefined) => {
      kept.text = value;
    },
  };
  return kept;
}

const persistence = (storage = fakeStorage()) =>
  new ViewPersistence({ views: new ViewMemory(storage), diffMode: "unified" });

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const at = (top: number, caret: number) => ({ top, caret });

const view = (): SavedView => ({
  open: ["changes", "file:a.rs"],
  active: "file:a.rs",
  modes: new Map([
    ["changes", "unified"],
    ["file:a.rs", "split"],
  ]),
  panes: new Map([["file:a.rs", { diff: at(5, 5) }]]),
});

void test("a pane's position is kept until the view it waits for is built, then taken once", () => {
  const p = persistence();
  assert.equal(p.takePending("file:a.rs", "diff"), undefined, "nothing is kept yet");
  p.keepPlace("file:a.rs", "diff", at(10, 3));
  p.keepPlace("file:a.rs", "text", at(2, 1));
  assert.deepEqual(p.pendingOf("file:a.rs", "diff"), at(10, 3));
  assert.deepEqual(p.takePending("file:a.rs", "diff"), at(10, 3));
  assert.equal(p.takePending("file:a.rs", "diff"), undefined, "a position is taken once");
  assert.deepEqual(p.pendingOf("file:a.rs", "text"), at(2, 1), "the other pane's position stays");

  p.keepPlace("file:b.rs", "diff", undefined);
  assert.equal(p.pendingOf("file:b.rs", "diff"), undefined, "nothing is kept for a pane with no position");
});

void test("the positions of the changeset brought back replace what was pending", () => {
  const p = persistence();
  p.keepPlace("file:a.rs", "diff", at(1, 1));
  p.setPending(new Map([["file:b.rs", { text: at(9, 9) }]]));
  assert.equal(p.pendingOf("file:a.rs", "diff"), undefined, "the old positions are gone");
  assert.deepEqual(p.pendingOf("file:b.rs", "text"), at(9, 9));

  p.setPending(undefined);
  assert.equal(p.pendingOf("file:b.rs", "text"), undefined);
});

void test("only a diff tab is brought back at a scroll position, and each once", () => {
  const p = persistence();
  p.beginRestore(["changes", "file:a.rs", "log:1"]);
  assert.equal(p.takeRestore("changes"), 0);
  assert.equal(p.takeRestore("file:a.rs"), 0);
  assert.equal(p.takeRestore("log:1"), undefined, "a log tab has no scroll position");
  assert.equal(p.takeRestore("changes"), undefined, "a position is taken once");
});

void test("the tabs and positions of a changeset survive a restart", () => {
  const storage = fakeStorage();
  persistence(storage).remember("pr:1", view());
  assert.deepEqual(persistence(storage).stored("pr:1"), view());
  assert.equal(persistence().stored("pr:1"), undefined, "another repository has none");
});

void test("a save waits and the last one wins; a cancelled save does not run", async () => {
  const p = persistence();
  let saves = 0;
  p.schedule(() => saves++);
  p.schedule(() => saves++);
  p.cancelScheduled();
  await sleep(500);
  assert.equal(saves, 0, "a cancelled save does not run");

  p.schedule(() => saves++);
  await sleep(500);
  assert.equal(saves, 1, "only the last scheduled save runs");
});
