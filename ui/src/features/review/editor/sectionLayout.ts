// Height of a file section estimated from the manifest alone, so a column of sections
// can be laid out before any text is loaded (SPEC §10). With `collapse`, unchanged regions
// between hunks become one bar each, mirroring `collapseUnchanged` of `@codemirror/merge`.
// Change cards for non-text items sit between the section header and the first hunk.
// A fixed gap closes every section.

import type { FileChange } from "../../../generated/FileChange";
import type { NonTextItem } from "../../../generated/NonTextItem";

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

/** Rows and bars an unchanged region of `lines` lines takes, with `context` lines kept at its edges. */
function unchanged(lines: number, context: number): { rows: number; bars: number } {
  return lines - context >= COLLAPSE.minSize ? { rows: context, bars: 1 } : { rows: Math.max(0, lines), bars: 0 };
}

/** `mode` is the editor's `Mode`; spelled out so this module needs no DOM or CodeMirror types. */
export function sectionLayout(
  file: FileChange,
  mode: "unified" | "split",
  m: Metrics,
  collapse: boolean,
): SectionLayout {
  const { margin } = COLLAPSE;
  const hunkTops: number[] = [];
  const cardTops: number[] = [];
  let y = m.header;
  for (const item of file.nonText) {
    cardTops.push(y);
    y += cardHeight(item, file.path, m);
  }
  const cardsHeight = y - m.header;
  let end = 0; // where the previous hunk ends in the target, in lines
  const gap = (lines: number, context: number) => {
    const u = collapse ? unchanged(lines, context) : { rows: Math.max(0, lines), bars: 0 };
    y += u.rows * m.line + u.bars * m.collapsed;
  };
  file.hunks.forEach((h, i) => {
    gap(h.targetStart - end, i === 0 ? margin : 2 * margin);
    hunkTops.push(y);
    // Unified shows each deleted line as an extra row; side by side pads the shorter side.
    const rows = mode === "unified" ? h.baseLen + h.targetLen : Math.max(h.baseLen, h.targetLen);
    y += m.hunk + rows * m.line;
    end = h.targetStart + h.targetLen;
  });
  gap(file.targetLines - end, file.hunks.length ? margin : 0);
  return { height: y + m.gap, cardTops, cardsHeight, hunkTops };
}
