// Where a file's text shows its changes (the File view): which target lines a hunk added or
// changed, where lines were removed, which words differ on a modified line, and how strongly a
// hunk's review state paints. Pure: the editor extensions draw what this works out.

import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import type { TextChange } from "../../../generated/TextChange";
import { type LineDoc, lineOf } from "./positions.ts";

/** How strongly a hunk's lines and words are tinted: what is left to review is, what was seen is faint. */
export type Tint = "full" | "faint";

export const tintOf = (state: HunkState): Tint => (state === "seen" || state === "informational" ? "faint" : "full");

/** A range of one line, in columns of that line. */
export interface Columns {
  from: number;
  to: number;
}

/** A range of the document that a hunk's modified line has changed words in. */
export interface WordRange {
  from: number;
  to: number;
  hunk: number;
}

/** Lines a hunk removed, which have no line of their own in the target: they are anchored between two lines. */
export interface Deletion {
  hunk: number;
  /** The 0-based target line the removal sits above; the number of lines when it sits below the last one. */
  line: number;
}

export interface FileLayout {
  /** The hunk that added or changed each 0-based target line. */
  lineHunk: ReadonlyMap<number, number>;
  deletions: readonly Deletion[];
  /** The changed words of modified lines, in document order. */
  words: readonly WordRange[];
}

/** What of CodeMirror's `Text` is read besides lines. */
export interface TextDoc extends LineDoc {
  line(n: number): { from: number; to: number; text: string };
}

/**
 * The pieces of the document range `from`..`to` that a line-wise view marks: one per line the
 * range touches, leaving out the break between lines and every line the range covers whole, which
 * the line's own tint already marks.
 */
function wordPieces(doc: TextDoc, from: number, to: number): { line: number; from: number; to: number }[] {
  const out: { line: number; from: number; to: number }[] = [];
  if (to <= from) return out;
  for (let line = lineOf(doc, from); line <= lineOf(doc, to - 1) && line < doc.lines; line++) {
    const l = doc.line(line + 1);
    const [a, b] = [Math.max(from, l.from), Math.min(to, l.to)];
    if (b > a && !(a <= l.from && b >= l.to)) out.push({ line, from: a, to: b });
  }
  return out;
}

export function fileLayout(hunks: readonly Hunk[], changes: readonly TextChange[], target: TextDoc): FileLayout {
  const lineHunk = new Map<number, number>();
  const deletions: Deletion[] = [];
  hunks.forEach((h, hunk) => {
    if (h.targetLen === 0) {
      deletions.push({ hunk, line: Math.min(h.targetStart, target.lines) });
      return;
    }
    for (let line = h.targetStart; line < Math.min(h.targetStart + h.targetLen, target.lines); line++)
      lineHunk.set(line, hunk);
  });
  const words: WordRange[] = [];
  for (const c of changes) {
    for (const piece of wordPieces(target, c.fromTarget, c.toTarget)) {
      const hunk = lineHunk.get(piece.line);
      // An added line is changed whole; words are told apart on a line that was modified.
      if (hunk !== undefined && hunks[hunk].baseLen > 0) words.push({ from: piece.from, to: piece.to, hunk });
    }
  }
  return { lineHunk, deletions, words };
}

/**
 * The line whose edge a removal's wedge sits on: the bottom of the line above it (so it stays at the
 * top of the removal's peek), or the top of the first line when the lines were removed from the start.
 */
export function wedgeAt(d: Deletion, lines: number): { line: number; at: "above" | "below" } {
  return { line: Math.min(Math.max(d.line - 1, 0), lines - 1), at: d.line === 0 ? "above" : "below" };
}

/** The hunk whose lines hold the 0-based `line`; none for a line the changeset left alone. */
export function hunkAtLine(layout: FileLayout, line: number): number | undefined {
  return layout.lineHunk.get(line);
}

/**
 * The hunk that hunk navigation reaches from `current` (-1: none) `delta` stops on. From none it
 * starts at the caret: the first hunk below its line going forward, the last one above going back.
 * Navigation stops at the first and last hunk.
 */
export function stepHunk(hunks: readonly Hunk[], current: number, caretLine: number, delta: number): number {
  if (!hunks.length) return -1;
  if (current < 0) {
    const below = hunks.findIndex((h) => h.targetStart > caretLine);
    if (delta > 0) return below < 0 ? hunks.length - 1 : below;
    const above = (below < 0 ? hunks.length : below) - 1;
    return Math.max(above, 0);
  }
  return Math.max(0, Math.min(hunks.length - 1, current + delta));
}

/** A line of the base text, with the columns of it that the change replaced. */
export interface OldLine {
  /** 0-based line of the base text. */
  line: number;
  text: string;
  marks: Columns[];
}

/**
 * The old text of the changed target `line` of `hunk`: its own line when the hunk replaced as many
 * lines as it holds, else every line the hunk replaced. A line that was added has none. The
 * replaced words come from the changes that fall in the old lines.
 */
export function oldLinesOf(hunk: Hunk, line: number, changes: readonly TextChange[], base: TextDoc): OldLine[] {
  if (hunk.baseLen === 0 || hunk.targetLen === 0) return [];
  const first = hunk.baseLen === hunk.targetLen ? hunk.baseStart + (line - hunk.targetStart) : hunk.baseStart;
  const count = hunk.baseLen === hunk.targetLen ? 1 : hunk.baseLen;
  const out: OldLine[] = [];
  for (let n = first; n < Math.min(first + count, base.lines); n++) {
    const l = base.line(n + 1);
    const marks: Columns[] = [];
    for (const c of changes)
      for (const piece of wordPieces(base, c.fromBase, c.toBase))
        if (piece.line === n) marks.push({ from: piece.from - l.from, to: piece.to - l.from });
    out.push({ line: n, text: l.text, marks });
  }
  return out;
}

/** A line cut where its replaced words start and end. */
export interface Segment {
  text: string;
  marked: boolean;
}

export function segments(text: string, marks: readonly Columns[]): Segment[] {
  const out: Segment[] = [];
  let at = 0;
  for (const m of [...marks].sort((a, b) => a.from - b.from)) {
    const from = Math.max(m.from, at);
    if (from > at) out.push({ text: text.slice(at, from), marked: false });
    if (m.to > from) out.push({ text: text.slice(from, m.to), marked: true });
    at = Math.max(at, m.to);
  }
  if (at < text.length) out.push({ text: text.slice(at), marked: false });
  return out;
}

/** The lines a pure deletion removed, from the base text. */
export function removedLines(hunk: Hunk, base: TextDoc): { number: number; text: string }[] {
  const out: { number: number; text: string }[] = [];
  for (let n = hunk.baseStart; n < Math.min(hunk.baseStart + hunk.baseLen, base.lines); n++)
    out.push({ number: n + 1, text: base.line(n + 1).text });
  return out;
}
