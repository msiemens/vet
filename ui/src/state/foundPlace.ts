// What the Find tool lists: places of text, whether Find in Files or Find usages found them, as one
// shape. A place carries what a row shows and what opening it and the preview need; the review state
// is the hunk's as it was when the set ran.

import type { Completion } from "../generated/Completion.ts";
import type { FileStatus } from "../generated/FileStatus.ts";
import type { Hunk } from "../generated/Hunk.ts";
import type { HunkState } from "../generated/HunkState.ts";
import type { LocationRow } from "../generated/LocationRow.ts";
import type { SearchFile } from "../generated/SearchFile.ts";
import type { SearchHit } from "../generated/SearchHit.ts";
import type { SearchLineKind } from "../generated/SearchLineKind.ts";
import type { Side } from "../generated/Side.ts";
import { codeRuns, libraryLabel, segments, trimmed } from "../lib/foundText.ts";
import { NO_HIGHLIGHTS } from "../lib/highlights.ts";
import { fullName, placeLabel } from "../lib/symbolLabel.ts";

/** The state of the hunk a place is in: seen, seen against an old base, not seen, or no hunk. */
export type Review = "seen" | "old" | "unseen" | "none";

export function reviewOf(state: HunkState | undefined): Review {
  switch (state) {
    case "seen":
    case "informational":
      return "seen";
    case "seenAgainstOldBase":
      return "old";
    case undefined:
      return "none";
    default:
      return "unseen";
  }
}

/** A stretch of a place's line, coloured as the syntax has it, with the match marked. */
export interface Run {
  text: string;
  className?: string;
  hit: boolean;
}

/** What is known of a file the places are in. */
export interface FoundFile {
  path: string;
  /** How the changeset changes the file; none for a file it leaves alone. */
  status?: FileStatus;
  completion?: Completion;
  /** The hunks of a changed file, with the review state of each. */
  hunks: Hunk[];
  states: HunkState[];
  /** A dependency's or toolchain's file, which has no snapshot and cannot be opened. */
  external: boolean;
  /** The path as a header shows it: a library's file by its package. */
  label: string;
}

export interface Place {
  /** The same for the same place in a later run of the same set. */
  id: string;
  file: FoundFile;
  /** 0-based, in the text of `side`. */
  line: number;
  /** Where the first match starts in the line and how wide it is, in UTF-16 units. */
  col: number;
  width: number;
  /** Which text the line is of: a place in old code is the base's. */
  side: Side;
  /** A removed line is the base's, an added one the target's changed, any other is unchanged. */
  kind: SearchLineKind;
  /** The line as the row shows it, with the match marked. */
  runs: readonly Run[];
  /** The line, trimmed, which the filter reads. */
  text: string;
  /** The matches in the line, in its own columns. */
  hits: readonly SearchHit[];
  /** What the place is in, short; a place Find in Files found has none. */
  within?: string;
  /** The enclosing symbol in full, for the tooltip. */
  withinTitle?: string;
  review: Review;
  /** The line itself is changed. */
  changed: boolean;
}

export const idOf = (side: Side, path: string, line: number, col: number) => `${side}:${path}:${line}:${col}`;

/** The places of the files a search found: one per matching line. */
export function placesFromSearch(files: readonly SearchFile[]): Place[] {
  return files.flatMap((f) => {
    const file: FoundFile = {
      path: f.path,
      status: f.status,
      completion: f.completion,
      hunks: f.hunks,
      states: f.states,
      external: false,
      label: f.path,
    };
    return f.lines.map((line): Place => {
      const { text, hits } = trimmed(line);
      const first = line.hits[0]?.from ?? 0;
      const side: Side = line.kind === "removed" ? "base" : "target";
      return {
        id: idOf(side, f.path, line.line, first),
        file,
        line: line.line,
        col: first,
        width: (line.hits[0]?.to ?? first) - first,
        side,
        kind: line.kind,
        text,
        hits: line.hits,
        runs: segments(text, 0, NO_HIGHLIGHTS, hits).map((s) => ({
          text: s.text,
          className: s.className,
          hit: !!s.find,
        })),
        review: reviewOf(line.hunk === undefined ? undefined : f.states[line.hunk]),
        changed: line.kind !== "context",
      };
    });
  });
}

/** What the changeset says of a changed file, if it is one. */
export type ChangedFile = Pick<FoundFile, "status" | "completion" | "hunks" | "states">;

/** The places a navigation request found, on `side`; `changed` says what the changeset knows of a file. */
export function placesFromRows(
  rows: readonly LocationRow[],
  side: Side,
  changed: (path: string) => ChangedFile | undefined,
): Place[] {
  const files = new Map<string, FoundFile>();
  return rows.map((row): Place => {
    let file = files.get(row.path);
    if (!file) {
      const known = row.external ? undefined : changed(row.path);
      const lib = row.external ? libraryLabel(row.path) : undefined;
      file = {
        path: row.path,
        status: known?.status,
        completion: known?.completion,
        hunks: known?.hunks ?? [],
        states: known?.states ?? [],
        external: row.external,
        label: lib ? `${lib.package ? `${lib.package} › ` : ""}${lib.rest}` : row.path,
      };
      files.set(row.path, file);
    }
    const line = row.line - 1;
    const col = row.col - 1;
    const inHunk = row.change === "hunk";
    const width = row.matchEnd - row.matchStart;
    return {
      id: idOf(side, row.path, line, col),
      file,
      line,
      col,
      width,
      side,
      kind: inHunk ? (side === "base" ? "removed" : "added") : "context",
      text: row.text,
      hits: [{ from: col, to: col + width }],
      runs: codeRuns(row),
      within: placeLabel(row.within),
      withinTitle: row.within ? fullName(row.within.id) : undefined,
      review: row.hunk === undefined ? "none" : reviewOf(file.states[row.hunk]),
      changed: inHunk,
    };
  });
}
