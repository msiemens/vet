// Marks in a view's scroll bar (`lib/ui/scrollbar.ts`): one per hunk, coloured by its review state,
// so the whole file or changeset can be read at a glance. The diff views and a file's text share
// the kinds; each says where its hunks lie.

import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import type { Mode } from "../../../lib/viewMode.ts";
import type { MarkLayout, ScrollMark } from "../../../lib/ui/scrollbar.ts";
import type { Metrics } from "./sectionLayout.ts";

/** What a mark says of its hunk: flagged for later, seen, still to review, seen against an old base, or lines removed and still to review. */
export type MarkKind = "flagged" | "seen" | "unseen" | "baseChanged" | "deletion";

/** A flag outranks the state: it is what the viewer set out to come back to. */
export function markKind(state: HunkState, deletion: boolean, flagged: boolean): MarkKind {
  if (flagged) return "flagged";
  switch (state) {
    case "seen":
    case "informational":
      return "seen";
    case "seenAgainstOldBase":
      return "baseChanged";
    case "unseen":
    case "unapplied":
      return deletion ? "deletion" : "unseen";
  }
}

/** What of a file's section the marks of a diff view need. */
export interface MarkedSection {
  top: number;
  /** Estimated offset of each hunk bar from the top of the section. */
  hunkTops: readonly number[];
  file: { path: string; hunks: readonly Hunk[] };
}

/** What a mark needs of a hunk's review state. */
export interface MarkedReview {
  hunks: readonly HunkState[];
  flagged: readonly boolean[];
}

/** The px a hunk of a diff takes below its bar: its lines on both sides, or the taller side. */
function hunkRows(h: Hunk, mode: Mode, line: number): number {
  return (mode === "split" ? Math.max(h.baseLen, h.targetLen) : h.baseLen + h.targetLen) * line;
}

/**
 * The marks of a column of file sections (the Changes tab, or one file's tab), from the height model.
 * `review` gives a file's state as it is when the marks are drawn.
 */
export function sectionMarks(
  sections: readonly MarkedSection[],
  review: (path: string) => MarkedReview | undefined,
  total: number,
  mode: Mode,
  m: Pick<Metrics, "hunk" | "line">,
): MarkLayout {
  const marks: ScrollMark[] = [];
  for (const s of sections) {
    const state = review(s.file.path);
    s.file.hunks.forEach((h, i) =>
      marks.push({
        top: s.top + s.hunkTops[i],
        height: m.hunk + hunkRows(h, mode, m.line),
        kind: markKind(state?.hunks[i] ?? "unseen", h.targetLen === 0, state?.flagged[i] ?? false),
      }),
    );
  }
  return { total, marks };
}
