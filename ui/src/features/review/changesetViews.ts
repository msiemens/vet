// What the review screen remembers of a changeset while another is on view: its open tabs, their
// view modes, the scroll position of each, and the hunk it was at. Kept in memory for the project's
// screen.

import type { Mode, ViewMode } from "../../lib/viewMode";
import type { HunkAnchor } from "./editor/ChangesetView";
import { CHANGES, type TabId, isStale } from "./tabModel.ts";

export interface SavedView {
  open: TabId[];
  active: TabId;
  /** The view mode of each tab that has one. */
  modes: ReadonlyMap<TabId, ViewMode>;
  /** The scroll position of each tab's pane. */
  scroll: ReadonlyMap<TabId, number>;
  /** The current hunk of the Changes tab. */
  anchor?: HunkAnchor;
}

/**
 * The tabs a changeset comes back with: those it had that still have something to show (see
 * `isStale`; `changed` says which files the changeset changes), the Changes tab first, and the one
 * that was active unless it is gone. A changeset seen for the first time has the Changes tab alone,
 * in the diff mode `diffMode`.
 */
export function restoredTabs(saved: SavedView | undefined, changed: (path: string) => boolean, diffMode: Mode) {
  const open = saved ? saved.open.filter((tab) => !isStale(tab, saved.modes.get(tab), changed)) : [CHANGES];
  const active = saved && open.includes(saved.active) ? saved.active : CHANGES;
  const modes = new Map<TabId, ViewMode>();
  for (const tab of open) {
    const mode = saved?.modes.get(tab) ?? (tab === CHANGES ? diffMode : undefined);
    if (mode !== undefined) modes.set(tab, mode);
  }
  return { open, active, modes };
}
