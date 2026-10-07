// Pure position logic of the editors: protocol positions (0-based line, UTF-16 column) to editor
// offsets.

import type { Position } from "../../../generated/Position";

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
