import assert from "node:assert/strict";
import { test } from "node:test";
import type { FileChange } from "../../../generated/FileChange.ts";
import { sectionLayout } from "./sectionLayout.ts";

const metrics = { line: 20, hunk: 24, header: 26, collapsed: 27 };
const hunk = (targetStart: number, baseLen: number, targetLen: number) => ({
  baseStart: targetStart,
  baseLen,
  targetStart,
  targetLen,
});
const file = (targetLines: number, hunks: ReturnType<typeof hunk>[]) => ({ targetLines, hunks }) as FileChange;
const layout = (f: FileChange, mode: "unified" | "split", collapse = true) => sectionLayout(f, mode, metrics, collapse);

void test("unchanged regions collapse to one bar each, keeping 3 lines of context", () => {
  const f = file(100, [hunk(50, 2, 3)]);
  // header, 3 lines + bar, hunk bar, 2 deleted + 3 added lines, 3 lines + bar.
  assert.deepEqual(layout(f, "unified"), { height: 26 + 87 + 24 + 100 + 87, hunkTops: [113] });
  // Side by side pads the shorter side: 3 rows instead of 5.
  assert.equal(layout(f, "split").height, 26 + 87 + 24 + 60 + 87);
});

void test("a gap too small to hide lines stays open", () => {
  const l = layout(file(30, [hunk(10, 1, 1), hunk(17, 1, 1)]), "unified");
  assert.deepEqual(l, { height: 448, hunkTops: [113, 297] });
});

void test("a file without hunks is one bar, or just the header when empty", () => {
  assert.equal(layout(file(40, []), "unified").height, 26 + 27);
  assert.equal(layout(file(0, []), "unified").height, 26);
});

void test("without collapsing every line is a row", () => {
  const l = layout(file(100, [hunk(50, 2, 3)]), "unified", false);
  // header, 100 target lines, 2 deleted lines, one hunk bar; the bar sits after 50 lines.
  assert.deepEqual(l, { height: 26 + 100 * 20 + 2 * 20 + 24, hunkTops: [26 + 50 * 20] });
});
