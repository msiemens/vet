// Double-clicking a line of a diff opens the file's text tab at the target line it stands for.

import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import type { Hunk } from "../../../generated/Hunk";
import { lineOf, targetLineOfBase } from "./positions";

/** The text of the lines, not the hunk bars, cards and buttons drawn among them. */
const LINES = ".cm-line, .cm-deletedChunk";

/**
 * Opens a 0-based target line on a double click on a line of the editor. `side` is the text the
 * editor holds; in a unified diff the deleted lines are widgets, which stand for the target line
 * their chunk sits before.
 */
export function openOnDoubleClick(
  side: "base" | "target",
  hunks: readonly Hunk[],
  open: (line: number) => void,
): Extension {
  return EditorView.domEventHandlers({
    dblclick: (event, view) => {
      const target = event.target instanceof Element ? event.target : undefined;
      const el = target?.closest(LINES);
      if (!el || target?.closest("button")) return false;
      const chunk = el.closest(".cm-deletedChunk");
      const pos = chunk ? view.posAtDOM(chunk) : view.posAtCoords({ x: event.clientX, y: event.clientY }, false);
      const line = lineOf(view.state.doc, pos);
      open(side === "base" ? targetLineOfBase(hunks, line) : line);
      return false;
    },
  });
}
