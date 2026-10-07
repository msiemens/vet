// What is pinned at the top of a `ChangesetView` at a scroll position: the file whose header is
// stuck and the hunk whose bar is pinned under it (`stickyHunk.ts` draws it).

import { sectionAt } from "./sectionHeights.ts";

export interface Pinned {
  /** The section whose header is pinned. */
  file: number;
  /** The hunk whose bar is pinned under it; none until the scroll position is past the first hunk's bar. */
  hunk?: number;
  /** How far up the bar has been pushed by the next hunk's, in px: 0 or negative. */
  push: number;
}

/**
 * The file and hunk pinned with the top of the viewport at `y`: the last section that starts at or
 * above `y`, and its last hunk whose bar starts at or above the line under the pinned header (of
 * height `header`). The next hunk's bar, `bar` px tall, pushes the pinned one up as it arrives.
 */
export function pinnedAt(
  sections: readonly { top: number; hunkTops: readonly number[] }[],
  y: number,
  header: number,
  bar: number,
): Pinned | undefined {
  if (!sections.length || y < sections[0].top) return undefined;
  const file = sectionAt(sections, y);
  const { top, hunkTops } = sections[file];
  const under = y + header - top; // the line under the header, from the section's top
  const hunk = lastAtOrBelow(hunkTops, under);
  if (hunk < 0) return { file, push: 0 };
  const next = hunkTops[hunk + 1] as number | undefined;
  return { file, hunk, push: next === undefined ? 0 : Math.min(0, next - under - bar) };
}

/** The index of the last of the ascending `values` that is at or below `limit`; -1 if there is none. */
function lastAtOrBelow(values: readonly number[], limit: number): number {
  let lo = 0;
  let hi = values.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (values[mid] <= limit) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}
