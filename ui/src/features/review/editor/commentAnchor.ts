// Where a new comment goes: Ctrl+Alt+C and the editor's menu comment on the
// selection, else on the caret's line; a hunk's Comment button on the whole hunk, or on the
// selection when it lies inside that hunk. Pure: the views hand in the editor they remember.

import type { CommentAt } from "../../../generated/CommentAt";
import type { Hunk } from "../../../generated/Hunk";
import type { Side } from "../../../generated/Side";
import { type LineDoc, selectedLines } from "./positions.ts";

/** The main selection of the editor the reviewer was in last, and the side its text is of. */
export interface EditorSelection {
  side: Side;
  doc: LineDoc;
  from: number;
  to: number;
}

/** The selected lines, else the caret's line. */
export function linesAt({ side, doc, from, to }: EditorSelection): CommentAt {
  return { type: "lines", side, ...selectedLines(doc, from, to) };
}

/** A hunk's lines on one side, as a 0-based start and a count (none on a side the hunk only adds to or removes from). */
const hunkLines = (h: Hunk, side: Side) =>
  side === "base" ? { start: h.baseStart, len: h.baseLen } : { start: h.targetStart, len: h.targetLen };

/** A hunk's Comment button: the selection when it overlaps the hunk's lines on its side, else the whole hunk. A caret alone does not narrow it. */
export function hunkComment(hunks: readonly Hunk[], index: number, selection?: EditorSelection): CommentAt {
  const whole: CommentAt = { type: "hunk", index };
  if (!selection || selection.from === selection.to) return whole;
  const lines = selectedLines(selection.doc, selection.from, selection.to);
  const h = hunkLines(hunks[index], selection.side);
  const overlaps = h.len > 0 && lines.start < h.start + h.len && h.start < lines.start + lines.len;
  return overlaps ? { type: "lines", side: selection.side, ...lines } : whole;
}

/**
 * Ctrl+Alt+C in a changed file's text (the target side): the selection or the caret's line. The
 * caret at a removal that navigation or its wedge chose stands for the removed lines, which have no
 * line of their own: that comments on the hunk, and so does an editor the reviewer was never in.
 * None when there is neither a selection nor a current hunk.
 */
export function fileViewComment(
  hunks: readonly Hunk[],
  current: number,
  selection?: EditorSelection,
): CommentAt | undefined {
  const atHunk: CommentAt | undefined = current >= 0 ? { type: "hunk", index: current } : undefined;
  if (!selection) return atHunk;
  if (selection.from === selection.to && current >= 0 && hunks[current].targetLen === 0) return atHunk;
  return linesAt(selection);
}
