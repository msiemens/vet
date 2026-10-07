// Pure position logic of the editors: protocol positions (0-based line, UTF-16 column) to editor
// offsets, and base lines to the target lines a unified diff shows them at.

import type { Hunk } from "../../../generated/Hunk";
import type { Position } from "../../../generated/Position";
import type { Side } from "../../../generated/Side";

/** What of CodeMirror's `Text` is needed: 1-based lines. */
export interface LineDoc {
  lines: number;
  line(n: number): { from: number; to: number };
}

/** The offset of a protocol position, clamped into the document (an answer may outlive an edit). */
export function offsetAt(doc: LineDoc, at: Position): number {
  if (at.line >= doc.lines) return doc.line(doc.lines).to;
  const line = doc.line(at.line + 1);
  return Math.min(line.from + at.col, line.to);
}

/**
 * The target line that line `line` of `side` faces in a unified diff: itself on the target side;
 * on the base side the last target line of the hunk it was deleted or replaced in, else the line
 * it shifted to. It places what the viewer does before the backend has an object to ask about (the
 * form for a new comment) and the landing of a go to definition. Comment threads come with the
 * line from the backend, which applies the same rule (`vet_core::review::target_line`).
 */
export function targetLine(hunks: readonly Hunk[], side: Side, line: number): number {
  if (side === "target") return line;
  let shift = 0;
  for (const h of hunks) {
    if (line < h.baseStart) break;
    if (line < h.baseStart + h.baseLen) return h.targetStart + Math.max(h.targetLen, 1) - 1;
    shift += h.targetLen - h.baseLen;
  }
  return Math.max(0, line + shift);
}
