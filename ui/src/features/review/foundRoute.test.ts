import assert from "node:assert/strict";
import { test } from "node:test";
import type { SearchScope } from "../../generated/SearchScope";
import type { View } from "../../generated/View";
import { routeFound } from "./foundRoute.ts";
const CHANGESET: View = { type: "changeset" };
const COMMIT_1: View = { type: "commit", index: 1 };
const COMMIT_2: View = { type: "commit", index: 2 };
const VERSIONS: View = { type: "interdiff", from: 1, to: 2 };
const OTHER_VERSIONS: View = { type: "interdiff", from: 2, to: 3 };
const searched = (view: View): SearchScope => ({ type: "view", view });
const REPOSITORY: SearchScope = { type: "repository" };

void test("a result of the changeset is shown in the changeset, never pinned to the commit on screen", () => {
  assert.deepEqual(routeFound(searched(CHANGESET), CHANGESET, false), { inChanges: true, pinned: false });
  // The Changes tab shows the commit, so the result opens in the file's own tab, which reads the changeset.
  assert.deepEqual(routeFound(searched(CHANGESET), COMMIT_1, false), { inChanges: false, pinned: false });
  assert.deepEqual(routeFound(searched(CHANGESET), VERSIONS, false), {
    show: CHANGESET,
    inChanges: true,
    pinned: false,
  });
});

void test("a result of a commit is shown pinned to that commit, in the Changes tab only while it is on screen", () => {
  assert.deepEqual(routeFound(searched(COMMIT_1), COMMIT_1, false), { inChanges: true, pinned: true });
  assert.deepEqual(routeFound(searched(COMMIT_1), COMMIT_2, false), { inChanges: false, pinned: true });
  assert.deepEqual(routeFound(searched(COMMIT_1), CHANGESET, false), { inChanges: false, pinned: true });
  assert.deepEqual(routeFound(searched(COMMIT_1), VERSIONS, false), { inChanges: false, pinned: true });
});

void test("a result of some versions brings those versions on screen, whatever else was", () => {
  assert.deepEqual(routeFound(searched(VERSIONS), VERSIONS, false), { inChanges: true, pinned: false });
  for (const onShow of [CHANGESET, COMMIT_1, OTHER_VERSIONS])
    assert.deepEqual(routeFound(searched(VERSIONS), onShow, false), { show: VERSIONS, inChanges: true, pinned: false });
});

void test("a result of the repository has a tab of its own, in a view that has the file's text", () => {
  for (const onShow of [CHANGESET, COMMIT_1])
    assert.deepEqual(routeFound(REPOSITORY, onShow, false), { inChanges: false, pinned: false });
  assert.deepEqual(routeFound(REPOSITORY, VERSIONS, false), { show: CHANGESET, inChanges: false, pinned: false });
});

void test("a result asked for in a tab of its own is never shown in the Changes tab", () => {
  const scopes: SearchScope[] = [searched(CHANGESET), searched(COMMIT_1), searched(VERSIONS), REPOSITORY];
  const shown: View[] = [CHANGESET, COMMIT_1, VERSIONS];
  for (const scope of scopes)
    for (const onShow of shown) assert.equal(routeFound(scope, onShow, true).inChanges, false);
});
