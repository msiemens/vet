import assert from "node:assert/strict";
import { test } from "node:test";
import { isCurrent } from "./shown.ts";
import { CHANGESET, commitView, interdiffView } from "./view.ts";

void test("a request built for the view that is on view goes out", () => {
  assert.equal(isCurrent({ revision: 4, view: commitView(1) }, { revision: 4, view: commitView(1) }), true);
  assert.equal(isCurrent({ revision: 4, view: interdiffView(1, 3) }, { revision: 4, view: interdiffView(1, 3) }), true);
});

void test("a request is refused once another view, another range or a newer revision is on view", () => {
  const shown = { revision: 4, view: interdiffView(1, 3) };
  assert.equal(isCurrent(shown, { revision: 4, view: CHANGESET }), false);
  assert.equal(isCurrent(shown, { revision: 4, view: interdiffView(2, 3) }), false);
  assert.equal(isCurrent(shown, { revision: 4, view: interdiffView(1, 4) }), false);
  assert.equal(isCurrent(shown, { revision: 5, view: interdiffView(1, 3) }), false);
  assert.equal(isCurrent(shown, undefined), false);
});

void test("stepping to the next commit refuses a mark from the commit still on screen until the next one has loaded", () => {
  const revision = 7;
  const onScreen = { revision, view: commitView(0) };
  // `]` asked for commit 1: the store's view moved, the views on screen are still commit 0's.
  const asked = { revision, view: commitView(1) };
  assert.equal(isCurrent(onScreen, asked), false, "Space in the old commit");
  // The manifest of commit 1 arrived and the views were rebuilt for it.
  assert.equal(isCurrent({ revision, view: commitView(1) }, asked), true, "Space in the new commit");
});
