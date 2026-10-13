// What a tab is: its identity and its view mode. A file has one tab per commit it is opened at (and
// one that is at no commit), whatever it shows of the file; the view mode (a diff in a mode, or the
// file's text) is the tab's state.

import type { TabId, TabKind } from "../../state/commands.ts";
import type { Mode, ViewMode } from "../../lib/viewMode.ts";

export const CHANGES: TabId = "changes";

/** The view on screen, as the origin of a request that no tab makes: the file tree, a panel. */
export const ON_SCREEN = "screen";

/** What a request to open a file starts from: a tab, whose view it reads, or `ON_SCREEN`. */
export type Origin = TabId | typeof ON_SCREEN;

/** The tab of a file; with `commit` (a full id), the one that stays at that commit. */
export const fileTab = (path: string, commit?: string): TabId =>
  commit === undefined ? `file:${path}` : `pin:${commit}:${path}`;
/** The tab of the log of the check run `run`. */
export const logTab = (run: string): TabId => `log:${run}`;

export const isFileTab = (id: TabId): boolean => id.startsWith("file:") || id.startsWith("pin:");

export const kindOf = (id: TabId): TabKind => (id === CHANGES ? "changes" : isFileTab(id) ? "file" : "log");

/** The tab can show the changeset's diff: the Changes tab, or one file's. */
export const hasDiff = (id: TabId): boolean => kindOf(id) !== "log";

/** What the Changes tab's tooltip says: how many files it lists. */
export const changesTitle = (files: number): string => `${files} changed ${files === 1 ? "file" : "files"}`;

/** The path of a file tab; `undefined` for the Changes tab and a log. */
export const pathOf = (id: TabId): string | undefined => {
  if (id.startsWith("file:")) return id.slice("file:".length);
  if (id.startsWith("pin:")) return id.slice(id.indexOf(":", "pin:".length) + 1);
  return undefined;
};

/** The full id of the commit a file tab stays at; none for any other tab. */
export const commitOf = (id: TabId): string | undefined =>
  id.startsWith("pin:") ? id.slice("pin:".length, id.indexOf(":", "pin:".length)) : undefined;

/** Whether the file of the pinned tab `id` is open in another tab too, so its commit has to be told apart by name. */
export const needsCommitName = (open: readonly TabId[], id: TabId): boolean => {
  const path = pathOf(id);
  return commitOf(id) !== undefined && open.some((other) => other !== id && pathOf(other) === path);
};

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

/**
 * The scroll offset of a strip showing `[scroll, scroll + view)` that brings the tab `[start, start + size)`
 * fully into view with the least movement; a tab wider than the view lines up its start.
 */
export function scrollToReveal(scroll: number, view: number, start: number, size: number): number {
  if (start < scroll || size > view) return start;
  if (start + size > scroll + view) return start + size - view;
  return scroll;
}
