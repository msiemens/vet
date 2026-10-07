import assert from "node:assert/strict";
import { test } from "node:test";
import { EditorState, StateField } from "@codemirror/state";
import { lineNumberMarkers } from "@codemirror/view";
import { longLines, setColumns } from "./longLines.ts";
import type { ReviewState } from "./reviewState.ts";

// Six lines of 5, 12, 12, 30, 4 and 12 characters; the hunk changes lines 1 to 3 (0-based).
const lines = ["a".repeat(5), "b".repeat(12), "c".repeat(12), "d".repeat(30), "e".repeat(4), "f".repeat(12)];
const doc = lines.join("\n");
const start = (line: number) => lines.slice(0, line).reduce((pos, l) => pos + l.length + 1, 0);

const review = StateField.define<ReviewState>({
  create: () => ({
    spans: [{ from: start(1), to: start(3) + lines[3].length, empty: false, at: start(1) }],
    states: ["unseen"],
    current: null,
    decos: undefined as never,
  }),
  update: (value) => value,
});

function marked(state: EditorState): number[] {
  const out: number[] = [];
  for (const set of state.facet(lineNumberMarkers)) {
    set.between(0, state.doc.length, (from) => {
      out.push(from);
    });
  }
  return out;
}

void test("a changed line that is wider than the code column is marked, an unchanged one is not", () => {
  const state = EditorState.create({ doc, extensions: [review, longLines(review, 10)] });
  // Lines 1, 2 and 3 are changed and wider than 10; line 5 is as wide but not changed.
  assert.deepEqual(marked(state), [start(1), start(2), start(3)]);
});

void test("a narrower column marks more of the changed lines, and a wider one fewer", () => {
  const state = EditorState.create({ doc, extensions: [review, longLines(review, 20)] });
  assert.deepEqual(marked(state), [start(3)]);
  const wider = state.update({ effects: setColumns.of(40) }).state;
  assert.deepEqual(marked(wider), []);
  const narrower = wider.update({ effects: setColumns.of(11) }).state;
  assert.deepEqual(marked(narrower), [start(1), start(2), start(3)]);
});
