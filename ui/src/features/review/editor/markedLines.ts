// Which lines of a file's text carry a change mark, and what a mark's hunk removed. The text is the
// target's, so a hunk that adds or changes lines marks those; one that only removes lines marks the
// edge of the line where they were.

import type { Hunk } from "../../../generated/Hunk";

/**
 * How a line is marked: `bar` is a line the hunk added or changed; `removedAbove` and `removedBelow`
 * mark where the hunk's removed lines were (above the line the hunk's position names, or below the
 * last line when they were at the end of the file).
 */
export type MarkKind = "bar" | "removedAbove" | "removedBelow";

export interface LineMark {
  hunk: number;
  kind: MarkKind;
}

/** The mark of each 0-based line of a text of `lines` lines. A bar wins over a removal on the same line. */
export function markedLines(hunks: readonly Hunk[], lines: number): Map<number, LineMark> {
  const marks = new Map<number, LineMark>();
  hunks.forEach((h, hunk) => {
    if (h.targetLen > 0) return;
    if (h.targetStart < lines) marks.set(h.targetStart, { hunk, kind: "removedAbove" });
    else if (lines > 0) marks.set(lines - 1, { hunk, kind: "removedBelow" });
  });
  hunks.forEach((h, hunk) => {
    for (let line = h.targetStart; line < Math.min(h.targetStart + h.targetLen, lines); line++)
      marks.set(line, { hunk, kind: "bar" });
  });
  return marks;
}

/** The lines the hunk removed or replaced, from the base text. */
export function oldLines(base: string, hunk: Pick<Hunk, "baseStart" | "baseLen">): string[] {
  return base
    .split("\n")
    .slice(hunk.baseStart, hunk.baseStart + hunk.baseLen)
    .map((line) => line.replace(/\r$/, ""));
}
