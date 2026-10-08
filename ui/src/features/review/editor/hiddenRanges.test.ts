import assert from "node:assert/strict";
import { test } from "node:test";
import { unifiedMergeView } from "@codemirror/merge";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { collapsedRegions, outsideHidden } from "./hiddenRanges.ts";

void test("the regions @codemirror/merge collapses are found from its own decorations", () => {
  const lines = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`);
  const original = lines.join("\n");
  lines[9] = "line ten";
  const state = EditorState.create({
    doc: lines.join("\n"),
    extensions: unifiedMergeView({ original, collapseUnchanged: { margin: 3, minSize: 4 } }),
  });
  const sets = state.facet(EditorView.decorations).filter((source) => typeof source !== "function");
  const hidden = collapsedRegions(sets, state.doc.length).map(({ from, to }) => [
    state.doc.lineAt(from).number,
    state.doc.lineAt(to).number,
  ]);
  assert.deepEqual(hidden, [
    [1, 6],
    [14, 20],
  ]);
});

// A document whose lines 10..19 are collapsed: positions 100..199 are behind the bar.
const hidden = [{ from: 100, to: 199 }];

void test("a range outside every hidden region is kept as it is", () => {
  assert.deepEqual(outsideHidden({ from: 20, to: 40 }, hidden), [{ from: 20, to: 40 }]);
  assert.deepEqual(outsideHidden({ from: 250, to: 260 }, hidden), [{ from: 250, to: 260 }]);
});

void test("a range inside a hidden region is dropped", () => {
  assert.deepEqual(outsideHidden({ from: 120, to: 150 }, hidden), []);
  assert.deepEqual(outsideHidden({ from: 100, to: 199 }, hidden), []);
});

void test("a range that runs into a hidden region keeps only the part outside, so the bar is not marked", () => {
  assert.deepEqual(outsideHidden({ from: 80, to: 130 }, hidden), [{ from: 80, to: 99 }]);
  assert.deepEqual(outsideHidden({ from: 150, to: 230 }, hidden), [{ from: 200, to: 230 }]);
});

void test("a range across a hidden region is split around it", () => {
  assert.deepEqual(outsideHidden({ from: 90, to: 210 }, hidden), [
    { from: 90, to: 99 },
    { from: 200, to: 210 },
  ]);
});

void test("a range of no length survives outside and is dropped inside", () => {
  assert.deepEqual(outsideHidden({ from: 50, to: 50 }, hidden), [{ from: 50, to: 50 }]);
  assert.deepEqual(outsideHidden({ from: 100, to: 100 }, hidden), []);
  assert.deepEqual(outsideHidden({ from: 199, to: 199 }, hidden), []);
  assert.deepEqual(outsideHidden({ from: 200, to: 200 }, hidden), [{ from: 200, to: 200 }]);
});

void test("several hidden regions, given in any order, are all cut out", () => {
  const regions = [
    { from: 300, to: 399 },
    { from: 100, to: 199 },
  ];
  assert.deepEqual(outsideHidden({ from: 0, to: 500 }, regions), [
    { from: 0, to: 99 },
    { from: 200, to: 299 },
    { from: 400, to: 500 },
  ]);
});

void test("nothing is hidden: the range is kept", () => {
  assert.deepEqual(outsideHidden({ from: 5, to: 9 }, []), [{ from: 5, to: 9 }]);
});
