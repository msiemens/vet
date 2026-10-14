// The rows of the timeline's table: one per version, newest first, and the base under them.

import type { CommitInfo } from "../../generated/CommitInfo";
import type { LineDelta } from "../../generated/LineDelta";
import type { VersionInfo } from "../../generated/VersionInfo";
import { basename } from "../../lib/paths.ts";
import { sumDelta } from "../../lib/ui/lineDelta.ts";
import { type Range, pushesOf } from "../../state/timelineRange.ts";

/** A file the range changes, as the panel lists it. */
export interface RangeFile {
  path: string;
  delta: LineDelta;
  /** Set when the file's mode changed. */
  mode?: "+x" | "-x" | "mode";
}

export interface TimelineRow {
  n: number;
  /** Unix seconds. */
  at: number;
  /** The commit the version was pushed as, whose checks the row shows. */
  commit?: string;
  /** The short names of the files the version changed, comma separated. */
  files: string;
  /** The lines the version changed; none when it changed no line. */
  lines?: LineDelta;
  /** The version something was last marked in. */
  reviewed: boolean;
  /** The push's changes are among those the review view shows. */
  shown: boolean;
  /** The version the changes shown are compared against. */
  ring: boolean;
}

/** What the rows say about the base the review started from, which no push changed. */
export interface BaseRow {
  commit: CommitInfo;
  /** The changes shown are compared against it. */
  ring: boolean;
}

/** What the rows mark: the pushes shown and the version they are compared against. */
export interface Shown {
  /** The range the review view shows; none for the whole changeset. */
  range?: Range;
  /** A commit is shown: no push is, whatever the range says. */
  byCommit?: boolean;
}

/** The rows for `versions` (oldest first), newest first, with what `shown` shows marked. */
export function timelineRows(versions: readonly VersionInfo[], { range, byCommit = false }: Shown = {}): TimelineRow[] {
  const pushes = range && pushesOf(range);
  const ringAt = range?.from ?? 0;
  return versions
    .map((v): TimelineRow => {
      const { added, removed } = sumDelta(v.files.map((f) => f.delta));
      return {
        n: v.n,
        at: v.at,
        commit: v.tip?.id,
        files: v.files.map((f) => basename(f.path)).join(", "),
        lines: added || removed ? { added, removed } : undefined,
        reviewed: v.reviewedHere,
        shown: !byCommit && (!pushes || (v.n >= pushes.first && v.n <= pushes.last)),
        ring: !byCommit && v.n === ringAt,
      };
    })
    .reverse();
}

/** What the timeline's header says is shown, `latest` being the newest version (none before a review). */
export function timelineSummary({ range, byCommit = false }: Shown, latest: number | undefined): string {
  if (latest === undefined) return "";
  if (byCommit) return "Showing one commit. Choosing a push leaves By commit.";
  if (!range) return "Showing every push";
  const { first, last } = pushesOf(range);
  return first === last ? `Showing what push v${first} changed` : `Showing pushes v${first} to v${last}`;
}

/**
 * The row of the base the review starts from. The whole changeset is compared with `current`, the
 * base now (a rebase can have moved it); a range from the start with the base the first version
 * was made against, if the versions say what it is.
 */
export function baseRow(
  versions: readonly VersionInfo[],
  current: CommitInfo | undefined,
  { range, byCommit = false }: Shown = {},
): BaseRow | undefined {
  const commit = range ? (versions.at(0)?.base ?? current) : current;
  return commit ? { commit, ring: !byCommit && (range?.from ?? 0) === 0 } : undefined;
}
