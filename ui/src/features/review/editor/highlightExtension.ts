// Highlights as mark decorations of an editor, built for the rendered part of the document only.

import { type Extension, RangeSetBuilder } from "@codemirror/state";
import { Decoration, type DecorationSet, type EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { type Highlights, eachRange, mapHighlights } from "./highlights";

const marks = new Map<string, Decoration>();
const markOf = (className: string) => {
  let mark = marks.get(className);
  if (!mark) marks.set(className, (mark = Decoration.mark({ class: className })));
  return mark;
};

/** Colours the editor's text with `initial`, which follows the document through edits. */
export function highlightExtension(initial: Highlights): Extension {
  if (!initial.from.length) return [];
  return ViewPlugin.fromClass(
    class {
      highlights = initial;
      decorations: DecorationSet;

      constructor(view: EditorView) {
        this.decorations = this.build(view);
      }

      update(update: ViewUpdate) {
        if (update.docChanged)
          this.highlights = mapHighlights(this.highlights, (pos, assoc) => update.changes.mapPos(pos, assoc));
        if (update.docChanged || update.viewportChanged) this.decorations = this.build(update.view);
      }

      build(view: EditorView): DecorationSet {
        const builder = new RangeSetBuilder<Decoration>();
        for (const { from, to } of view.visibleRanges)
          eachRange(this.highlights, from, to, (start, end, className) => builder.add(start, end, markOf(className)));
        return builder.finish();
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );
}
