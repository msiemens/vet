// What the review screen remembers of each changeset it has shown and across restarts: its open tabs,
// their view modes and where each pane was left, kept per repository. It also holds the positions a
// pane is put at when its view is rebuilt, and the scroll offsets a changeset is brought back at.

import { Debounced } from "../../lib/debounced.ts";
import type { TabId } from "../../state/commands";
import { type FilePositions, type SavedView, type Storage, ViewMemory } from "./changesetViews.ts";
import type { PanePosition } from "./editor/ChangesetView";
import { hasDiff } from "./tabModel.ts";

/** The tabs and positions are saved this long after the last change. */
const SAVE_AFTER_MS = 400;

export class ViewPersistence {
  /** Where file tabs' panes are put when they are built: the positions of the changeset just brought back. */
  private pending = new Map<TabId, FilePositions>();
  /** Scroll positions the next builds start at, instead of carrying the old views' over. */
  private restore?: { scroll: Map<TabId, number> };
  /** What each changeset of the repository had open when it was last on view, for when it comes back. */
  private memory?: ViewMemory;
  private saving = new Debounced();
  private readonly storage: Storage;

  constructor(storage: Storage) {
    this.storage = storage;
  }

  /** Remembers where a pane was, for the view that replaces it once its source has read. */
  keepPlace(id: TabId, pane: keyof FilePositions, at: PanePosition | undefined) {
    if (at) this.pending.set(id, { ...this.pending.get(id), [pane]: at });
  }

  /** The position a pane is put at when it is built, once. */
  takePending(id: TabId, pane: keyof FilePositions): PanePosition | undefined {
    const positions = this.pending.get(id);
    const at = positions?.[pane];
    if (positions) {
      positions[pane] = undefined;
      if (!positions.diff && !positions.text) this.pending.delete(id);
    }
    return at;
  }

  /** The position kept for a pane, without taking it. */
  pendingOf(id: TabId, pane: keyof FilePositions): PanePosition | undefined {
    return this.pending.get(id)?.[pane];
  }

  /** Replaces the pending positions with `panes`, the tabs of the changeset just brought back. */
  setPending(panes: ReadonlyMap<TabId, FilePositions> | undefined) {
    this.pending.clear();
    for (const [id, at] of panes ?? []) this.pending.set(id, { ...at });
  }

  /** Puts every diff tab at the top; a remembered position is put on it once it is built. */
  beginRestore(ids: Iterable<TabId>) {
    this.restore = { scroll: new Map([...ids].filter(hasDiff).map((tab) => [tab, 0])) };
  }

  /** The scroll a rebuild of the tab starts at, if a position is being restored; taken once. */
  takeRestore(id: TabId): number | undefined {
    const restore = this.restore;
    if (!restore?.scroll.has(id)) return undefined;
    const top = restore.scroll.get(id) ?? 0;
    restore.scroll.delete(id);
    if (restore.scroll.size === 0) this.restore = undefined;
    return top;
  }

  /** Remembers the tabs and where the panes are for the changeset `key`. */
  remember(key: string, view: SavedView) {
    this.memory ??= new ViewMemory(this.storage);
    this.memory.set(key, view);
  }

  /** What was remembered of the changeset `key`, if anything. */
  stored(key: string): SavedView | undefined {
    this.memory ??= new ViewMemory(this.storage);
    return this.memory.get(key);
  }

  /** Saves the views once the changes pause. */
  schedule(save: () => void) {
    this.saving.schedule(SAVE_AFTER_MS, save);
  }

  /** Drops a save that waits. */
  cancelScheduled() {
    this.saving.cancel();
  }
}
