// What a tab is: its identity and its view mode. A file has one tab, whatever it shows of the file;
// the view mode (a diff in a mode, or the file's text) is the tab's state.

import type { TabKind } from "../../state/commands.ts";
import type { Mode, ViewMode } from "../../lib/viewMode.ts";

export type TabId = "changes" | `file:${string}` | `log:${string}`;

export const CHANGES: TabId = "changes";

export const fileTab = (path: string): TabId => `file:${path}`;
/** The tab of the log of the check run `run`. */
export const logTab = (run: string): TabId => `log:${run}`;

/** The tab of a kind (and, but for the Changes tab, what it shows: a file, or a check run for a log). */
export const tabOf = (kind: TabKind, subject?: string): TabId =>
  kind === "changes" || subject === undefined ? CHANGES : kind === "file" ? fileTab(subject) : logTab(subject);

export const kindOf = (id: TabId): TabKind => (id === CHANGES ? "changes" : id.startsWith("file:") ? "file" : "log");

/** The tab can show the changeset's diff: the Changes tab, or one file's. */
export const hasDiff = (id: TabId): boolean => kindOf(id) !== "log";

/** The path of a file tab; `undefined` for the Changes tab and a log. */
export const pathOf = (id: TabId): string | undefined => (kindOf(id) === "file" ? subjectOf(id) : undefined);

/** What a tab shows, to name it to `tabOf`: the path of a file tab, the run of a log; none for the Changes tab. */
export const subjectOf = (id: TabId): string | undefined =>
  id === CHANGES ? undefined : id.slice(id.indexOf(":") + 1);

/** The view modes a tab offers: the Changes tab and a changed file's show a diff; any file's tab shows its text. */
export function modesOf(kind: TabKind, changed: boolean): ViewMode[] {
  if (kind === "changes") return ["unified", "split"];
  if (kind === "file") return changed ? ["unified", "split", "file"] : ["file"];
  return [];
}

/**
 * The mode a file's tab shows after it is asked to show `want`: that one if the file has it, else
 * the tab's own, else (a tab just opened) the diff mode `last` chosen. An unchanged file has no diff.
 */
export function settleMode(a: { changed: boolean; want?: ViewMode; current?: ViewMode; last: Mode }): ViewMode {
  if (!a.changed) return "file";
  return a.want ?? a.current ?? a.last;
}

/**
 * Whether the tab has nothing left to show once the changeset changed: a diff of a file that
 * `changed` no longer reports. A file's text stays.
 */
export const isStale = (id: TabId, mode: ViewMode | undefined, changed: (path: string) => boolean): boolean => {
  const path = pathOf(id);
  return path !== undefined && mode !== "file" && !changed(path);
};

/** The tab `delta` places from `active` among the tabs in display order, wrapping around the ends. */
export function tabAfter<T>(open: readonly T[], active: T, delta: number): T {
  const at = open.indexOf(active);
  return open[(((at + delta) % open.length) + open.length) % open.length];
}
