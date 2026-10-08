// The line a press landed on (or a jump went to) stays marked until a press lands on another line,
// in this editor or any other: one line is marked across all the editors at a time. A section's
// editor is dropped when it is far from the viewport and built again when it comes back; the
// mark it held is drawn again then.

import { type Extension, StateEffect } from "@codemirror/state";
import { Decoration, type DecorationSet, type EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";

const mark = Decoration.line({ class: "vet-current-line" });
const unmark = StateEffect.define();

/** The editor that holds the mark. */
let marked: EditorView | undefined;

/** Where the mark is, by the element the editors of a section are built in: the editor of it and the 1-based line. */
const remembered = new WeakMap<HTMLElement, { editor: number; line: number }>();
/** The host whose editors hold the mark. */
let markedHost: HTMLElement | undefined;

/**
 * Marks the line of the caret. With a `host` (the element a section's editors are built in) the
 * mark outlives the editor: `editor` tells which of the host's editors this is.
 */
export function currentLine(host?: HTMLElement, editor = 0): Extension {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet = Decoration.none;

      constructor(readonly view: EditorView) {
        const was = host && remembered.get(host);
        if (was?.editor === editor && was.line <= view.state.doc.lines) {
          this.decorations = Decoration.set([mark.range(view.state.doc.line(was.line).from)]);
          marked = view;
        }
      }

      update(update: ViewUpdate) {
        if (update.transactions.some((t) => t.effects.some((e) => e.is(unmark)))) {
          this.decorations = Decoration.none;
          return;
        }
        if (update.selectionSet) {
          const { doc, selection } = update.state;
          const line = doc.lineAt(selection.main.head);
          this.decorations = Decoration.set([mark.range(line.from)]);
          if (marked && marked !== update.view) marked.dispatch({ effects: unmark.of(null) });
          marked = update.view;
          if (markedHost && markedHost !== host) remembered.delete(markedHost);
          markedHost = host;
          if (host) remembered.set(host, { editor, line: line.number });
        } else if (update.docChanged) this.decorations = this.decorations.map(update.changes);
      }

      destroy() {
        if (marked === this.view) marked = undefined;
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );
}
