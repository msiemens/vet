import assert from "node:assert/strict";
import { test } from "node:test";
import { pinnedAt } from "./pinned.ts";

// A 26px header and 24px hunk bars. Offsets are from the section's top, as the height model has them.
const sections = [
  { top: 100, hunkTops: [126, 300] },
  { top: 500, hunkTops: [] },
  { top: 700, hunkTops: [60] },
];
const pinned = (y: number) => pinnedAt(sections, y, 26, 24);

void test("nothing is pinned above the first section", () => {
  assert.equal(pinned(40), undefined);
  assert.equal(pinnedAt([], 0, 26, 24), undefined);
});

void test("the file is the last section that starts at or above the top of the view", () => {
  assert.equal(pinned(100)?.file, 0);
  assert.equal(pinned(499)?.file, 0);
  assert.equal(pinned(500)?.file, 1);
  assert.equal(pinned(900)?.file, 2);
});

void test("a hunk is pinned once its bar is at or under the pinned header, and until the next one's", () => {
  // The first bar sits at 226 on the page; the header covers 26px below the top of the view.
  assert.equal(pinned(150)?.hunk, undefined);
  assert.equal(pinned(200)?.hunk, 0);
  assert.equal(pinned(290)?.hunk, 0);
  // The second bar is at 400 on the page: from 374 on.
  assert.equal(pinned(374)?.hunk, 1);
  assert.equal(pinned(450)?.hunk, 1);
});

void test("the next hunk's bar pushes the pinned one up as it arrives, and only then", () => {
  assert.equal(pinned(250)?.push, 0);
  // 12px of the next bar (at 400) are under the header's lower edge: the pinned bar is 12px short.
  assert.equal(pinned(374 - 12)?.push, -12);
  assert.equal(pinned(374 - 24)?.push, 0);
  assert.equal(pinned(374)?.push, 0); // the second hunk is pinned now, with no next
});

void test("a file without hunks pins only its header", () => {
  assert.deepEqual(pinned(600), { file: 1, push: 0 });
});
