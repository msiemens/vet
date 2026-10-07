// The open tabs: the fixed Changes tab, one diff tab per opened changed file and one read-only
// text tab per opened unchanged file, in display order. The order is the array's, so
// drag-to-reorder only has to move an entry of `open`.

import type { TabKind } from "../../state/commands";

export type TabId = "changes" | `file:${string}` | `text:${string}`;

export const CHANGES: TabId = "changes";

export const fileTab = (path: string): TabId => `file:${path}`;
export const textTab = (path: string): TabId => `text:${path}`;

/** The tab of a kind (and, but for the Changes tab, a file). */
export const tabOf = (kind: TabKind, path?: string): TabId =>
  kind === "changes" || path === undefined ? CHANGES : kind === "file" ? fileTab(path) : textTab(path);

export const kindOf = (id: TabId): TabKind => (id === CHANGES ? "changes" : id.startsWith("file:") ? "file" : "text");

/** The path of a file or text tab; `undefined` for the Changes tab. */
export const pathOf = (id: TabId): string | undefined => (id === CHANGES ? undefined : id.slice(id.indexOf(":") + 1));

export class Tabs {
  open = $state<TabId[]>([CHANGES]);
  active = $state<TabId>(CHANGES);
  /** Whether each tab's view wraps long lines, as its view reports it; written when a view is built or toggled. */
  wrapped = $state<Record<string, boolean>>({});

  /** Opens the file's diff tab if needed and focuses it. */
  show(path: string) {
    this.focus(fileTab(path));
  }

  /** Opens the file's text tab if needed and focuses it. */
  showText(path: string) {
    this.focus(textTab(path));
  }

  private focus(id: TabId) {
    if (!this.open.includes(id)) this.open.push(id);
    this.active = id;
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
