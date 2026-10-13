// The open tabs: the fixed Changes tab, one tab per opened file (changed or not) and one read-only
// log tab per opened CI log, in display order. The order is the array's, so drag-to-reorder only has
// to move an entry of `open`. A tab of a file or of changes has a view mode, which changing never
// opens or closes a tab.

import { SvelteMap } from "svelte/reactivity";
import type { Mode, ViewMode } from "../../lib/viewMode";
import { CHANGES, type TabId, fileTab, isStale, logTab, tabAfter } from "./tabModel";

/** How a log tab is named. */
export interface LogTabInfo {
  title: string;
  /** What the tab's tooltip adds, such as the step that failed. */
  hint?: string;
}

export class Tabs {
  open = $state<TabId[]>([CHANGES]);
  active = $state<TabId>(CHANGES);
  /** The view mode of the Changes tab and of each file tab. */
  modes = new SvelteMap<TabId, ViewMode>();
  /** The names of the open log tabs. */
  logs = new SvelteMap<TabId, LogTabInfo>();
  /** The file the Changes tab is at: the one whose header is pinned at its top. */
  changesFile = $state<string>();

  constructor(changesMode: Mode) {
    this.modes.set(CHANGES, changesMode);
  }

  /** Opens the file's tab (the one at `commit`, a full id, if given) if needed, sets its view mode and focuses it. */
  showFile(path: string, mode: ViewMode, commit?: string) {
    const id = fileTab(path, commit);
    this.modes.set(id, mode);
    this.focus(id);
  }

  /** Opens the log's tab if needed and focuses it. */
  showLog(run: string, info: LogTabInfo) {
    const id = logTab(run);
    this.logs.set(id, info);
    this.focus(id);
  }

  private focus(id: TabId) {
    if (!this.open.includes(id)) this.open.push(id);
    this.active = id;
  }

  /** Focuses the tab `delta` places from the active one, wrapping around the ends. */
  step(delta: number) {
    this.active = tabAfter(this.open, this.active, delta);
  }

  /** The Changes tab cannot be closed. Focus moves to the neighbour on the right, else on the left. */
  close(id: TabId) {
    const index = this.open.indexOf(id);
    if (id === CHANGES || index < 0) return;
    this.open.splice(index, 1);
    this.modes.delete(id);
    if (this.active === id) this.active = this.open[Math.min(index, this.open.length - 1)];
  }

  /** Closes every tab but the Changes tab and `id`, and focuses `id`. */
  closeOthers(id: TabId) {
    for (const tab of this.open) if (tab !== CHANGES && tab !== id) this.modes.delete(tab);
    this.open = this.open.filter((tab) => tab === CHANGES || tab === id);
    this.active = id;
  }

  /** Closes the tabs that show a diff of a file `changed` no longer reports in the tab's view. */
  dropStale(changed: (id: TabId, path: string) => boolean) {
    for (const id of [...this.open]) if (isStale(id, this.modes.get(id), (path) => changed(id, path))) this.close(id);
  }
}
