// The rows the Changes tool's Excluded group opens to: each file the review leaves out with the reason, and how many more a reason leaves out than the manifest lists.

import type { ExcludedGroup } from "../../generated/ExcludedGroup";
import { shortRule } from "./emptyChanges.ts";

export interface ExcludedRow {
  /** The file's path, or how many more files its reason leaves out. */
  name: string;
  /** The file the row opens; none for the count of the rest. */
  path?: string;
  /** Why the review leaves it out, in a word or two. */
  why: string;
}

export function excludedRows(groups: readonly ExcludedGroup[]): ExcludedRow[] {
  return groups.flatMap((g) => {
    const why = g.by.map(shortRule).join(", ");
    return [
      ...g.files.map((f) => ({ name: f.path, path: f.path, why })),
      ...(g.count > g.files.length ? [{ name: `+ ${g.count - g.files.length} more`, why }] : []),
    ];
  });
}

/** The files the review leaves out, all groups together. */
export const excludedCount = (groups: readonly ExcludedGroup[]): number => groups.reduce((sum, g) => sum + g.count, 0);

/** The rows the tree adds after the files: none without excluded changes, the group's own, and while it is open what it holds. */
export const extraRows = (excluded: boolean, open: boolean, rows: readonly ExcludedRow[]): number =>
  excluded ? 1 + (open ? rows.length : 0) : 0;
