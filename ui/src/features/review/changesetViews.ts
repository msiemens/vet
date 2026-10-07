// What the review screen remembers of a changeset while another is on view: its open tabs, the
// scroll position of each, and the hunk it was at. Kept in memory for the project's screen.

import type { HunkAnchor } from "./editor/ChangesetView";
import type { TabId } from "./Tabs.svelte";

export interface SavedView {
  open: TabId[];
  active: TabId;
  /** The scroll position of each tab's pane. */
  scroll: ReadonlyMap<TabId, number>;
  /** The current hunk of the Changes tab. */
  anchor?: HunkAnchor;
}

/**
 * The tabs a changeset comes back with: those it had that still make sense (`keep`), the
 * Changes tab first, and the one that was active unless it is gone. A changeset seen for the
 * first time has the Changes tab alone.
 */
export function restoredTabs(saved: SavedView | undefined, keep: (tab: TabId) => boolean) {
  const changes: TabId = "changes";
  const open = saved ? saved.open.filter((tab) => tab === changes || keep(tab)) : [changes];
  const active = saved && open.includes(saved.active) ? saved.active : changes;
  return { open, active };
}
