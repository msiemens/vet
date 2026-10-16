import assert from "node:assert/strict";
import { test } from "node:test";
import type { SavedView, Storage } from "./changesetViews.ts";
import { reviewMemory } from "./reviewMemory.ts";
import { ViewPersistence } from "./ViewPersistence.ts";

const unstored: Storage = { read: () => undefined, write: () => undefined };

const view = (): SavedView => ({
  open: ["changes", "log:1"],
  active: "log:1",
  modes: new Map([["changes", "split"]]),
  panes: new Map(),
});

void test("the screen of the next session finds what the screen before it remembered", () => {
  const memory = reviewMemory(unstored);
  const first = new ViewPersistence(memory("/repo"));
  first.remember("branch:a", view());
  first.diffMode = "split";

  const next = new ViewPersistence(memory("/repo"));
  assert.deepEqual(next.stored("branch:a"), view(), "a log tab is not stored, only remembered by the window");
  assert.equal(next.diffMode, "split");
});

void test("another repository starts with nothing remembered", () => {
  const memory = reviewMemory(unstored);
  new ViewPersistence(memory("/repo")).remember("branch:a", view());
  assert.equal(new ViewPersistence(memory("/other")).stored("branch:a"), undefined);
});
