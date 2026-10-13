// Marks in a view's scroll bar (`lib/ui/scrollbar.ts`): one per hunk, coloured by its review state,
// so the whole file or changeset can be read at a glance. The diff views and a file's text share
// the kinds; each says where its hunks lie.

import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import type { Mode } from "../../../lib/viewMode.ts";
import type { FindMatch, FindPos } from "./findMatches.ts";
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

/** What a find's tick needs of a section. */
export interface TickSection extends MarkedSection {
  height: number;
  file: { path: string; hunks: readonly Hunk[]; targetLines: number };
}

/** The most ticks of one kind a find's scroll bar draws: matches that fall into the same share of the content are one tick. */
export const TICK_ROWS = 1000;

/**
 * The ticks of a find's matches in the scroll bar of a column of sections `total` px tall. A changed
 * line is placed by its hunk's row; an unchanged one by its share of the section, since the editor
 * that would tell is not always there. Matches that lie within the same 1/`TICK_ROWS` of the content
 * share a tick, so a find with a match on every line costs the bar as little as one with a few; the
 * current match keeps a tick of its own.
 */
export function findTicks(
  sections: readonly TickSection[],
  matches: (section: number) => readonly FindMatch[],
  current: FindPos | undefined,
  mode: Mode,
  m: Pick<Metrics, "hunk" | "line" | "header" | "gap">,
  total: number,
): ScrollMark[] {
  const ticks: ScrollMark[] = [];
  const row = Math.max(total / TICK_ROWS, 1);
  const drawn = new Set<number>();
  sections.forEach((s, section) => {
    for (const match of matches(section)) {
      const h = match.hunk >= 0 ? s.file.hunks[match.hunk] : undefined;
      let top: number;
      if (!h) {
        const body = Math.max(s.height - m.header - m.gap, 0);
        top = s.top + m.header + (match.line / Math.max(s.file.targetLines, 1)) * body;
      } else {
        const row =
          mode === "split"
            ? match.line - (match.kind === "removed" ? h.baseStart : h.targetStart)
            : match.kind === "removed"
              ? match.line - h.baseStart
              : h.baseLen + match.line - h.targetStart;
        top = s.top + s.hunkTops[match.hunk] + m.hunk + row * m.line;
      }
      const at = current?.section === section && current.order === match.order && current.from === match.from;
      if (!at) {
        const key = Math.floor(top / row);
        if (drawn.has(key)) continue;
        drawn.add(key);
      }
      ticks.push({ top, height: 0, kind: at ? "findCurrent" : "find" });
    }
  });
  return ticks;
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
