// The open tabs: the fixed Changes tab, one diff tab per opened changed file, one read-only
// text tab per opened unchanged file and one read-only log tab per opened CI log, in display
// order. The order is the array's, so drag-to-reorder only has to move an entry of `open`.

import { SvelteMap } from "svelte/reactivity";
import type { TabKind } from "../../state/commands";
import { swapTab, tabAfter } from "./tabOrder";

export type TabId = "changes" | `file:${string}` | `text:${string}` | `log:${string}`;

export const CHANGES: TabId = "changes";

export const fileTab = (path: string): TabId => `file:${path}`;
export const textTab = (path: string): TabId => `text:${path}`;
/** The tab of the log of the check run `run`. */
export const logTab = (run: string): TabId => `log:${run}`;

/** The tab of a kind (and, but for the Changes tab, what it shows: a file, or a check run for a log). */
export const tabOf = (kind: TabKind, subject?: string): TabId =>
  kind === "changes" || subject === undefined
    ? CHANGES
    : kind === "file"
      ? fileTab(subject)
      : kind === "log"
        ? logTab(subject)
        : textTab(subject);

export const kindOf = (id: TabId): TabKind =>
  id === CHANGES ? "changes" : id.startsWith("file:") ? "file" : id.startsWith("log:") ? "log" : "text";

/** The tab shows the changeset's diff: the Changes tab, or one file's. */
export const showsDiff = (id: TabId): boolean => kindOf(id) === "changes" || kindOf(id) === "file";

/** The path of a file or text tab; `undefined` for the Changes tab and a log. */
export const pathOf = (id: TabId): string | undefined =>
  id === CHANGES || kindOf(id) === "log" ? undefined : id.slice(id.indexOf(":") + 1);

/** What a tab shows, to name it to `tabOf`: the path of a file or text tab, the run of a log; none for the Changes tab. */
export const subjectOf = (id: TabId): string | undefined =>
  id === CHANGES ? undefined : id.slice(id.indexOf(":") + 1);

/** How a log tab is named. */
export interface LogTabInfo {
  title: string;
  /** What the tab's tooltip adds, such as the step that failed. */
  hint?: string;
}

export class Tabs {
  open = $state<TabId[]>([CHANGES]);
  active = $state<TabId>(CHANGES);
  /** The names of the open log tabs. */
  logs = new SvelteMap<TabId, LogTabInfo>();
  /** The file the Changes tab is at: the one whose header is pinned at its top. */
  changesFile = $state<string>();

  /** Opens the file's diff tab if needed and focuses it. */
  show(path: string) {
    this.focus(fileTab(path));
  }

  /** Opens the file's text tab if needed and focuses it. */
  showText(path: string) {
    this.focus(textTab(path));
  }

  /** The tab `to` takes the place of the open tab `from` and is focused. */
  swap(from: TabId, to: TabId) {
    this.open = swapTab(this.open, from, to);
    this.active = to;
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
    if (this.active === id) this.active = this.open[Math.min(index, this.open.length - 1)];
  }

  /** Closes every tab but the Changes tab and `id`, and focuses `id`. */
  closeOthers(id: TabId) {
    this.open = this.open.filter((tab) => tab === CHANGES || tab === id);
    this.active = id;
  }

  /** Closes the diff tabs whose file the changeset no longer contains. */
  keepOnly(paths: ReadonlySet<string>) {
    for (const id of [...this.open]) {
      if (kindOf(id) === "file" && !paths.has(pathOf(id) ?? "")) this.close(id);
    }
  }
}
