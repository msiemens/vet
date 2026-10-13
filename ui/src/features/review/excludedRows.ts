// The rows the Changes tool's Excluded group opens to: each file the review leaves out with the reason, and how many more a reason leaves out than the manifest lists.

import type { ExcludedGroup } from "../../generated/ExcludedGroup";
import { ruleName } from "./emptyChanges.ts";

export interface ExcludedRow {
  /** The file's path, or how many more files its reason leaves out. */
  name: string;
  /** Why the review leaves it out. */
  why: string;
}

export function excludedRows(groups: readonly ExcludedGroup[]): ExcludedRow[] {
  return groups.flatMap((g) => [
    ...g.files.map((f) => ({ name: f.path, why: ruleName(g.by) })),
    ...(g.count > g.files.length ? [{ name: `+ ${g.count - g.files.length} more`, why: ruleName(g.by) }] : []),
  ]);
}

/** The rows the tree adds after the files: none without excluded changes, the group's own, and while it is open what it holds. */
export const extraRows = (excluded: boolean, open: boolean, rows: readonly ExcludedRow[]): number =>
  excluded ? 1 + (open ? rows.length : 0) : 0;
