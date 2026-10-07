// How many rows the lines of a file take when the editor wraps them (SPEC §10: soft wrap). The
// manifest knows only line counts, so a section is laid out at one row per line until its text
// has arrived; from the text and the width of the code column the rows of every line follow, and
// CodeMirror's own measure corrects what this estimates (`mount.ts` reports it).

import type { RowExtras } from "./sectionLayout.ts";

/** Width of the review strip and of the merge extension's change gutter, in px. */
const STRIPS = 3 + 4;
/** The narrowest a line number column gets (`section.css`), and what it adds to the digits. */
const NUMBER_MIN = 40;
const NUMBER_PAD = 14;
/** What `.cm-line` pads on both sides. */
const LINE_PAD = 10;

/** The columns of `text[from, to)`: a tab advances to the next multiple of `tabSize`. */
export function columnsOf(text: string, from: number, to: number, tabSize: number): number {
  let columns = 0;
  for (let i = from; i < to; i++) columns += text.charCodeAt(i) === 9 ? tabSize - (columns % tabSize) : 1;
  return columns;
}

/**
 * How many characters of a line fit in one row of the code column, for a view `width` px wide
 * (`mode` splits it into two panes) with characters `char` px wide and a file of `lines` lines,
 * which decides the width of the line number column.
 */
export function rowColumns(width: number, mode: "unified" | "split", char: number, lines: number): number {
  const pane = mode === "split" ? width / 2 : width;
  const numbers = Math.max(NUMBER_MIN, String(lines).length * char + NUMBER_PAD);
  return Math.max(1, Math.floor((pane - STRIPS - numbers - LINE_PAD) / char));
}

/** The extra rows (beyond one) each line of a text takes when it wraps at `columns`. */
export class WrapRows implements RowExtras {
  /** `extras[i]` is the extra rows of the lines before line `i`; one entry more than there are lines. */
  private extras: Int32Array;

  constructor(text: string, columns: number, tabSize = 4) {
    const extras: number[] = [0];
    let total = 0;
    for (let from = 0; ;) {
      const nl = text.indexOf("\n", from);
      const to = nl < 0 ? text.length : nl;
      const end = to > from && text.charCodeAt(to - 1) === 13 ? to - 1 : to;
      total += Math.max(0, Math.ceil(columnsOf(text, from, end, tabSize) / columns) - 1);
      extras.push(total);
      if (nl < 0) break;
      from = nl + 1;
    }
    this.extras = Int32Array.from(extras);
  }

  /** The extra rows of lines `[from, to)`, 0-based; lines past the end have none. */
  extra(from: number, to: number): number {
    const last = this.extras.length - 1;
    return this.extras[Math.min(to, last)] - this.extras[Math.min(from, last)];
  }
}
