// Pure position logic of the editors: protocol positions (0-based line, UTF-16 column) to editor
// offsets, and base lines to the target lines a unified diff shows them at.

import type { Hunk } from "../../../generated/Hunk";
import type { Position } from "../../../generated/Position";
import type { Side } from "../../../generated/Side";

/** What of CodeMirror's `Text` is needed: 1-based lines. */
export interface LineDoc {
  lines: number;
  line(n: number): { from: number; to: number };
  lineAt(pos: number): { number: number };
}

/** The offset where the 0-based `line` starts; a line past the end is the last one. */
export const lineStart = (doc: LineDoc, line: number): number => doc.line(Math.min(line + 1, doc.lines)).from;

/** The offset where the 0-based `line` ends; a line past the end is the last one. */
export const lineEnd = (doc: LineDoc, line: number): number => doc.line(Math.min(line + 1, doc.lines)).to;

/** The 0-based line that holds the offset. */
export const lineOf = (doc: LineDoc, pos: number): number => doc.lineAt(pos).number - 1;

/** The offset of a protocol position, clamped into the document (an answer may outlive an edit). */
export function offsetAt(doc: LineDoc, at: Position): number {
  if (at.line >= doc.lines) return lineEnd(doc, at.line);
  return Math.min(lineStart(doc, at.line) + at.col, lineEnd(doc, at.line));
}

/** The text range a selection covers, as whole lines: a selection ending at a line start does not include that line. */
export function selectedLines(
  startLine: number,
  endLine: number,
  endsAtLineStart: boolean,
): { start: number; len: number } {
  const last = endsAtLineStart && endLine > startLine ? endLine - 1 : endLine;
  return { start: startLine, len: last - startLine + 1 };
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
