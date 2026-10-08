// The rows of the timeline's table: one per version, newest first.

import type { LineDelta } from "../../generated/LineDelta";
import type { VersionInfo } from "../../generated/VersionInfo";
import { basename } from "../../lib/paths.ts";
import type { Range } from "../../state/timelineRange";

/** A file the range changes, as the panel lists it. */
export interface RangeFile {
  path: string;
  delta: LineDelta;
  /** Set when the file's mode changed. */
  mode?: "+x" | "-x" | "mode";
}

/** Where a row lies in the chosen range: an end of it, or between the ends. */
export type Band = "end" | "inside";

export interface TimelineRow {
  n: number;
  /** Unix seconds. */
  at: number;
  /** A move of the base: the files are not shown, the new base is. */
  rebasedOnto?: string;
  /** The commit the version was pushed as, whose checks the row shows; none for a move of the base. */
  commit?: string;
  /** The short names of the files the version changed, comma separated. */
  files: string;
  /** The lines the version changed; none when it changed no line. */
  lines?: LineDelta;
  /** The version something was last marked in. */
  reviewed: boolean;
  band?: Band;
}

/** The rows for `versions` (oldest first), newest first, with `range` marked. */
export function timelineRows(versions: readonly VersionInfo[], range?: Range): TimelineRow[] {
  return versions
    .map((v): TimelineRow => {
      const added = v.files.reduce((sum, f) => sum + f.delta.added, 0);
      const removed = v.files.reduce((sum, f) => sum + f.delta.removed, 0);
      return {
        n: v.n,
        at: v.at,
        rebasedOnto: v.origin === "baseMoved" ? (v.base?.shortId ?? "") : undefined,
        commit: v.origin === "baseMoved" ? undefined : v.tip?.id,
        files: v.files.map((f) => basename(f.path)).join(", "),
        lines: added || removed ? { added, removed } : undefined,
        reviewed: v.reviewedHere,
        band:
          range && v.n >= range.from && v.n <= range.to
            ? v.n === range.from || v.n === range.to
              ? "end"
              : "inside"
            : undefined,
      };
    })
    .reverse();
}
