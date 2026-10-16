// A changed line that runs past the width of the code column is marked in the line number gutter,
// whether the editor wraps it or scrolls it sideways, so the reviewer knows there is more of it
// than is in view (SPEC §8.3).

import { type EditorState, type Extension, type Range, RangeSet, StateEffect, StateField } from "@codemirror/state";
import { GutterMarker, lineNumberMarkers } from "@codemirror/view";
import type { ReviewState } from "./reviewState.ts";
import { columnsOf } from "./wrapRows.ts";

/** The characters that fit a row of the code column changed: the view was resized. */
export const setColumns = StateEffect.define<number>();

const long = new (class extends GutterMarker {
  elementClass = "vet-long";
})();

/** The markers of the changed lines of `review` that take more than `limit` characters. */
function marks(state: EditorState, review: StateField<ReviewState>, limit: number): RangeSet<GutterMarker> {
  const out: Range<GutterMarker>[] = [];
  for (const span of state.field(review).spans) {
    if (span.empty) continue;
    for (let pos = span.from; pos <= span.to;) {
      const line = state.doc.lineAt(pos);
      if (columnsOf(line.text, 0, line.length, state.tabSize) > limit) out.push(long.range(line.from));
      pos = line.to + 1;
    }
  }
  return RangeSet.of(out, true);
}

/**
 * Marks the lines of the changed hunks of `review` that take more than `columns` characters;
 * `setColumns` moves the limit. Comes after `review` in an editor's extensions.
 */
export function longLines(review: StateField<ReviewState>, columns: number): Extension {
  return StateField.define<RangeSet<GutterMarker>>({
    create: (state) => marks(state, review, columns),
    update(value, tr) {
      let limit: number | undefined;
      for (const effect of tr.effects) if (effect.is(setColumns)) limit = effect.value;
      return limit === undefined ? value : marks(tr.state, review, limit);
    },
    provide: (field) => lineNumberMarkers.from(field),
  });
}
