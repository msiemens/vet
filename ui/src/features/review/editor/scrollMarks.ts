// Marks along a view's scroll bar: one per hunk, coloured by its review state, and a band for the
// part that is in view, so the whole file or changeset can be read at a glance. The diff views and a
// file's text share it: each says where its hunks lie (`MarkLayout`), this draws them.

import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import type { Mode } from "../../../lib/viewMode.ts";
import type { Metrics } from "./sectionLayout.ts";

/** What a mark says of its hunk: seen, still to review, seen against an older base, or lines removed and still to review. */
export type MarkKind = "seen" | "unseen" | "baseChanged" | "deletion";

export function markKind(state: HunkState, deletion: boolean): MarkKind {
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

/** A hunk's place in the view's content, in px from its top. */
export interface ScrollMark {
  top: number;
  height: number;
  kind: MarkKind;
}

/** Where the hunks lie in a view whose content is `total` px tall. */
export interface MarkLayout {
  total: number;
  marks: readonly ScrollMark[];
}

/** A mark's smallest size on the track, so a one-line hunk of a long changeset can still be seen. */
export const MIN_MARK = 3;

export interface TrackRect {
  top: number;
  height: number;
}

/** The marks scaled to a track of `track` px, none smaller than `MIN_MARK` and none off the track. */
export function markRects(layout: MarkLayout, track: number): (TrackRect & { kind: MarkKind })[] {
  if (layout.total <= 0 || track <= 0) return [];
  const scale = track / layout.total;
  return layout.marks.map((m) => {
    const height = Math.min(Math.max(m.height * scale, MIN_MARK), track);
    const top = Math.min(Math.max(m.top * scale, 0), track - height);
    return { top, height, kind: m.kind };
  });
}

/** The part of the content in view, scaled to the track. */
export function bandRect(total: number, scrollTop: number, viewport: number, track: number): TrackRect {
  if (total <= 0 || track <= 0) return { top: 0, height: 0 };
  const scale = track / total;
  const height = Math.min(viewport * scale, track);
  return { top: Math.min(Math.max(scrollTop * scale, 0), track - height), height };
}

/** What of a file's section the marks of a diff view need. */
export interface MarkedSection {
  top: number;
  /** Estimated offset of each hunk bar from the top of the section. */
  hunkTops: readonly number[];
  file: { hunks: readonly Hunk[] };
  review: { hunks: readonly HunkState[] };
}

/** The px a hunk of a diff takes below its bar: its lines on both sides, or the taller side. */
function hunkRows(h: Hunk, mode: Mode, line: number): number {
  return (mode === "split" ? Math.max(h.baseLen, h.targetLen) : h.baseLen + h.targetLen) * line;
}

/** The marks of a column of file sections (the Changes tab, or one file's tab), from the height model. */
export function sectionMarks(
  sections: readonly MarkedSection[],
  total: number,
  mode: Mode,
  m: Pick<Metrics, "hunk" | "line">,
): MarkLayout {
  const marks: ScrollMark[] = [];
  for (const s of sections)
    s.file.hunks.forEach((h, i) =>
      marks.push({
        top: s.top + s.hunkTops[i],
        height: m.hunk + hunkRows(h, mode, m.line),
        kind: markKind(s.review.hunks[i], h.targetLen === 0),
      }),
    );
  return { total, marks };
}

/**
 * Draws a `MarkLayout` over the right edge of `host`, which is positioned and does not scroll with
 * `scroller`. It lets every pointer event through.
 */
export class ScrollMarks {
  private track = document.createElement("div");
  private band = document.createElement("div");
  private marks: HTMLElement[] = [];
  private frame?: number;
  private layout?: MarkLayout;
  private scroller: HTMLElement;
  private source: () => MarkLayout | undefined;

  constructor(host: HTMLElement, scroller: HTMLElement, source: () => MarkLayout | undefined) {
    this.scroller = scroller;
    this.source = source;
    this.track.className = "vet-scrollmarks";
    this.band.className = "vet-scrollmarks-band";
    this.track.append(this.band);
    host.append(this.track);
    scroller.addEventListener("scroll", this.schedule, { passive: true });
    this.schedule();
  }

  /** The hunks moved, changed state or appeared: draws them again in the next frame. */
  refresh = () => {
    this.layout = undefined;
    this.schedule();
  };

  private schedule = () => {
    this.frame ??= requestAnimationFrame(() => {
      this.frame = undefined;
      this.draw();
    });
  };

  private draw() {
    this.layout ??= this.source();
    const layout = this.layout;
    const track = this.track.clientHeight;
    const rects = layout ? markRects(layout, track) : [];
    while (this.marks.length < rects.length) {
      const mark = document.createElement("div");
      this.track.append(mark);
      this.marks.push(mark);
    }
    this.marks.forEach((mark, i) => {
      const rect = rects.at(i);
      mark.hidden = !rect;
      if (!rect) return;
      mark.className = `vet-scrollmark vet-scrollmark-${rect.kind}`;
      mark.style.top = `${rect.top}px`;
      mark.style.height = `${rect.height}px`;
    });
    const band = layout
      ? bandRect(layout.total, this.scroller.scrollTop, this.scroller.clientHeight, track)
      : { top: 0, height: 0 };
    this.band.hidden = band.height <= 0 || band.height >= track;
    this.band.style.top = `${band.top}px`;
    this.band.style.height = `${band.height}px`;
  }

  destroy() {
    if (this.frame !== undefined) cancelAnimationFrame(this.frame);
    this.scroller.removeEventListener("scroll", this.schedule);
    this.track.remove();
  }
}
