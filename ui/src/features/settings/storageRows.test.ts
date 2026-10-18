import assert from "node:assert/strict";
import { test } from "node:test";
import type { ProjectStorage } from "../../generated/ProjectStorage.ts";
import { largestFirst, projectTotal } from "./storageRows.ts";

function project(root: string, figures: Partial<ProjectStorage>): ProjectStorage {
  return {
    root,
    folderExists: true,
    open: false,
    recent: false,
    lastOpened: null,
    trusted: false,
    reviewData: 0,
    sourceTrees: 0,
    sourceTreesInUse: 0,
    semanticCache: 0,
    openReviews: 0,
    unpostedComments: 0,
    pendingVotes: 0,
    computedAt: 0,
    ...figures,
  };
}

void test("a project counts its review data, source trees and semantic cache", () => {
  assert.equal(projectTotal(project("/a", { reviewData: 1, sourceTrees: 20, semanticCache: 300 })), 321);
});

void test("the project that takes most comes first, whichever part makes it large", () => {
  const rows = largestFirst([
    project("/trees", { sourceTrees: 50 }),
    project("/answers", { semanticCache: 90 }),
    project("/review", { reviewData: 70 }),
  ]);
  assert.deepEqual(
    rows.map((p) => p.root),
    ["/answers", "/review", "/trees"],
  );
});

void test("projects that take the same are ordered by root", () => {
  const rows = largestFirst([project("/b", { reviewData: 5 }), project("/a", { sourceTrees: 5 })]);
  assert.deepEqual(
    rows.map((p) => p.root),
    ["/a", "/b"],
  );
});
