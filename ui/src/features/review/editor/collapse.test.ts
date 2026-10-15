import assert from "node:assert/strict";
import { test } from "node:test";
import { unifiedMergeView } from "@codemirror/merge";
import { type EditorState, EditorState as State } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { collapsedField, collapsedSpans, expandCollapsed, hiddenRegions, setCollapsed } from "./collapse.ts";

const COLLAPSE = { margin: 3, minSize: 4 };

/**
 * The regions `@codemirror/merge`'s own collapse hides, read from its decorations. Test-only: the
 * product reads its own `collapsedField`; this is the reference the owned ranges are checked against.
 */
function mergeRegions(state: EditorState) {
  const regions: { from: number; to: number }[] = [];
  for (const source of state.facet(EditorView.decorations)) {
    if (typeof source === "function") continue;
    source.between(0, state.doc.length, (from, to, deco) => {
      const widget = (deco.spec as { widget?: { type?: string } }).widget;
      if (widget?.type === "collapsed-unchanged-code") regions.push({ from, to });
    });
  }
  return regions;
}

void test("the owned collapse hides exactly the regions @codemirror/merge hides", () => {
  const lines = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`);
  const original = lines.join("\n");
  lines[9] = "line ten";
  const state = State.create({
    doc: lines.join("\n"),
    extensions: unifiedMergeView({ original, collapseUnchanged: COLLAPSE }),
  });
  const owned = collapsedSpans(state, COLLAPSE);
  assert.equal(owned.length, 2);
  assert.deepEqual(owned, mergeRegions(state));
});

void test("the collapse field maps its regions through a document change and removes one on an effect", () => {
  let state = State.create({ doc: "aaa\nbbb\nccc\nddd", extensions: collapsedField });
  assert.deepEqual(hiddenRegions(state), []);
  state = state.update({ effects: setCollapsed.of([{ from: 8, to: 11 }]) }).state;
  assert.deepEqual(hiddenRegions(state), [{ from: 8, to: 11 }]);
  // Text inserted above moves the region down by the text's length.
  state = state.update({ changes: { from: 0, insert: "X\n" } }).state;
  assert.deepEqual(hiddenRegions(state), [{ from: 10, to: 13 }]);
  // Expanding removes the named region and leaves the others.
  state = state.update({
    effects: setCollapsed.of([
      { from: 2, to: 5 },
      { from: 10, to: 13 },
    ]),
  }).state;
  state = state.update({ effects: expandCollapsed.of(10) }).state;
  assert.deepEqual(hiddenRegions(state), [{ from: 2, to: 5 }]);
});
