// The open tabs: the fixed Changes tab and one tab per opened file, in display order.
// The order is the array's, so drag-to-reorder only has to move an entry of `open`.

export type TabId = "changes" | `file:${string}`;

export const CHANGES: TabId = "changes";

export const fileTab = (path: string): TabId => `file:${path}`;

/** The path of a file tab; `undefined` for the Changes tab. */
export const pathOf = (id: TabId): string | undefined => (id === CHANGES ? undefined : id.slice("file:".length));

export class Tabs {
  open = $state<TabId[]>([CHANGES]);
  active = $state<TabId>(CHANGES);

  /** Opens the file's tab if needed and focuses it. */
  show(path: string) {
    const id = fileTab(path);
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

  /** Closes the file tabs whose file the changeset no longer contains. */
  keepOnly(paths: ReadonlySet<string>) {
    for (const id of [...this.open]) {
      const path = pathOf(id);
      if (path !== undefined && !paths.has(path)) this.close(id);
    }
  }
}
