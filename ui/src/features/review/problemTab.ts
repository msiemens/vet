// Which tab shows a problem that is picked in the Problems tool, without Jump to Source.

export type ProblemTab =
  /** The Changes tab, which is on screen and has the file. */
  | "changes"
  /** The file's own tab, which is on screen or open already. */
  | "file"
  /** The Changes tab, brought to the front for the file. */
  | "changesToFront"
  /** A tab the file gets. */
  | "new";

/**
 * The tab is the one on screen when it shows the file, else the file's open tab, else the Changes
 * tab if it has the file, else a new one. `inChanges` says the Changes tab shows the file in the
 * whole changeset's latest code; a tab at a commit is never chosen by the caller, so it is never here.
 */
export function problemTab(on: {
  active: string;
  changes: string;
  file: string;
  open: readonly string[];
  inChanges: boolean;
}): ProblemTab {
  if (on.active === on.changes && on.inChanges) return "changes";
  if (on.active === on.file || on.open.includes(on.file)) return "file";
  return on.inChanges ? "changesToFront" : "new";
}
