// The line a press landed on (or a jump went to) stays marked until a press lands on another line,
// in this editor or any other: one line is marked across all the editors at a time.

import { StateEffect } from "@codemirror/state";
import { Decoration, type DecorationSet, type EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";

const mark = Decoration.line({ class: "vet-current-line" });
const unmark = StateEffect.define();

/** The editor that holds the mark. */
let marked: EditorView | undefined;

export const currentLine = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet = Decoration.none;

    constructor(readonly view: EditorView) {}

    update(update: ViewUpdate) {
      if (update.transactions.some((t) => t.effects.some((e) => e.is(unmark)))) {
        this.decorations = Decoration.none;
        return;
      }
      if (update.selectionSet) {
        const { doc, selection } = update.state;
        this.decorations = Decoration.set([mark.range(doc.lineAt(selection.main.head).from)]);
        if (marked && marked !== update.view) marked.dispatch({ effects: unmark.of(null) });
        marked = update.view;
      } else if (update.docChanged) this.decorations = this.decorations.map(update.changes);
    }

    destroy() {
      if (marked === this.view) marked = undefined;
    }
  },
  { decorations: (plugin) => plugin.decorations },
);
