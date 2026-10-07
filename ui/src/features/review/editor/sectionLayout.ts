// Height of a file section estimated from the manifest alone, so a column of sections
// can be laid out before any text is loaded (SPEC §10). With `collapse`, unchanged regions
// between hunks become one bar each, mirroring `collapseUnchanged` of `@codemirror/merge`.
// Change cards for non-text items sit between the section header and the first hunk.
// A fixed gap closes every section.

import type { FileChange } from "../../../generated/FileChange";
import type { NonTextItem } from "../../../generated/NonTextItem";
import type { Side } from "../../../generated/Side";

/** Context kept around each hunk, and the smallest unchanged region worth a bar. */
export const COLLAPSE = { margin: 3, minSize: 4 };

/** Row heights in px; they come from the theme tokens. */
export interface Metrics {
  line: number;
  hunk: number;
  header: number;
  collapsed: number;
  /** Empty space below a section's last row, so it does not touch the next header. */
  gap: number;
  /** The before/after preview strip of an image card. */
  preview: number;
}

export interface SectionLayout {
  height: number;
  /** Offset of each change card from the top of the section. */
  cardTops: number[];
  /** Height of all cards together. */
  cardsHeight: number;
  /** Offset of each hunk's header bar from the top of the section. */
  hunkTops: number[];
  /** Which of the section's inline thread cards have a row to sit below, in the order given. */
  drawn: boolean[];
}

const IMAGE = /\.(png|jpe?g|gif|webp|bmp|ico|svg)$/i;

/** Text rows of a card: the before and after line, or one note when there is nothing to compare. */
export const cardRows = (item: NonTextItem): number => (item.kind === "contentUnavailable" ? 1 : 2);

/** Image files show a before/after preview below the lines. */
export const hasPreview = (item: NonTextItem, path: string): boolean =>
  item.kind === "binary" && IMAGE.test(path) && (item.before !== null || item.after !== null);

/** A card is a header bar like a hunk's, its rows, and the preview strip if it has one. */
export const cardHeight = (item: NonTextItem, path: string, m: Metrics): number =>
  m.hunk + cardRows(item) * m.line + (hasPreview(item, path) ? m.preview : 0);

/** Comment threads that add height to a section (see `threadPlacement.ts`). */
export interface ThreadSpace {
  /** The file-level threads' block, below the change cards. */
  block: number;
  /** Cards shown below a target line; one in a collapsed region is not drawn and adds nothing. */
  inline: { line: number; height: number }[];
}

/**
 * The lines of a side (0-based, `[from, to)`, in order) that have rows: all of them, or the hunks and
 * their context when unchanged regions collapse. What lies between the ranges is hidden behind bars.
 * The layout, the thread cards that are drawn and the size of a bar all derive from this.
 */
export function shownLines(file: FileChange, collapse: boolean, side: Side = "target"): [number, number][] {
  const total = side === "base" ? file.baseLines : file.targetLines;
  if (!collapse) return [[0, total]];
  const { margin } = COLLAPSE;
  const shown: [number, number][] = [];
  let end = 0;
  const gap = (to: number, head: number, tail: number) => {
    const lines = to - end;
    if (lines - head - tail >= COLLAPSE.minSize) {
      if (head) shown.push([end, end + head]);
      if (tail) shown.push([to - tail, to]);
    } else if (lines > 0) shown.push([end, to]);
  };
  file.hunks.forEach((h, i) => {
    const [start, len] = side === "base" ? [h.baseStart, h.baseLen] : [h.targetStart, h.targetLen];
    gap(start, i === 0 ? 0 : margin, margin);
    shown.push([start, start + len]);
    end = start + len;
  });
  gap(total, file.hunks.length ? margin : 0, 0);
  return shown;
}

/** How many lines the bar that starts at (or covers) `line` of `side` hides; 0 when `line` is shown. */
export function hiddenAt(file: FileChange, collapse: boolean, side: Side, line: number): number {
  const total = side === "base" ? file.baseLines : file.targetLines;
  let end = 0;
  const ranges: [number, number][] = [...shownLines(file, collapse, side), [total, total]];
  for (const [from, to] of ranges) {
    if (line >= end && line < from) return from - end;
    end = Math.max(end, to);
  }
  return 0;
}

/** `mode` is the editor's `Mode`; spelled out so this module needs no DOM or CodeMirror types. */
export function sectionLayout(
  file: FileChange,
  mode: "unified" | "split",
  m: Metrics,
  collapse: boolean,
  threads: ThreadSpace = { block: 0, inline: [] },
): SectionLayout {
  const hunkTops: number[] = [];
  const cardTops: number[] = [];
  let y = m.header;
  for (const item of file.nonText) {
    cardTops.push(y);
    y += cardHeight(item, file.path, m);
  }
  const cardsHeight = y - m.header;
  y += threads.block;
  const shown = shownLines(file, collapse);
  const drawn = threads.inline.map((c) => shown.some(([from, to]) => c.line >= from && c.line < to));
  const above = (line: number) =>
    threads.inline.reduce((sum, c, i) => (drawn[i] && c.line < line ? sum + c.height : sum), 0);
  let end = 0; // where the previous hunk ends in the target, in lines
  let next = 0; // the first shown range that can still reach the lines not yet measured
  /** The unchanged lines `[from, to)`: their shown rows, and one bar if some are hidden. */
  const gap = (from: number, to: number) => {
    let rows = 0;
    for (; next < shown.length && shown[next][0] < to; next++) {
      const [a, b] = shown[next];
      rows += Math.max(0, Math.min(b, to) - Math.max(a, from));
      if (b > to) break;
    }
    y += rows * m.line + (rows < to - from ? m.collapsed : 0);
  };
  file.hunks.forEach((h) => {
    gap(end, h.targetStart);
    hunkTops.push(y + above(h.targetStart));
    // Unified shows each deleted line as an extra row; side by side pads the shorter side.
    const rows = mode === "unified" ? h.baseLen + h.targetLen : Math.max(h.baseLen, h.targetLen);
    // Where the target side has no lines, the filler that aligns it counts the base side's bar too.
    const aligned = mode === "split" && h.targetLen === 0 ? m.hunk : 0;
    y += m.hunk + aligned + rows * m.line;
    end = h.targetStart + h.targetLen;
  });
  gap(end, file.targetLines);
  return { height: y + above(Infinity) + m.gap, cardTops, cardsHeight, hunkTops, drawn };
}
