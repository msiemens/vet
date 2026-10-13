// The review state of the files of one view, one reactive cell per path, and how it is written: the
// changeset store holds the view on screen's, a `ViewScope` the view a tab stays at. A write names the
// paths it concerns, or "all" for a manifest, which replaces every file; `ReviewOrder` says what a newer
// arrival may replace.

import { SvelteMap } from "svelte/reactivity";
import { untrack } from "svelte";
import type { FileState } from "./reviewModel";
import { type Newer, ReviewOrder } from "./reviewOrder";
import type { ReviewSource, WriteListener } from "./reviewSource";

class Cells implements ReviewSource {
  private cells = new SvelteMap<string, FileState>();
  private listeners = new Set<WriteListener>();
  /** When each path was last written, in writes counted from the start; never reset, so a path that comes back is newer. */
  private versions = new Map<string, number>();
  private clock = 0;
  private held = new ReviewOrder();

  /** Which change of the backend the state held is as of. */
  get order(): ReviewOrder {
    return this.held;
  }

  /** The state of a file as it is now, read reactively. */
  file(path: string): FileState | undefined {
    return this.cells.get(path);
  }

  has(path: string): boolean {
    return this.cells.has(path);
  }

  state(path: string): FileState | undefined {
    return untrack(() => this.cells.get(path));
  }

  version(path: string): number {
    return this.versions.get(path) ?? 0;
  }

  /** A manifest's states replace the held ones, but those `newer` says are newer than the manifest. */
  replace(states: ReadonlyMap<string, FileState>, newer: Newer) {
    const kept = new Map(newer.files.size ? this.cells : []);
    this.write(new Map([...states].map(([path, state]) => [path, kept.get(path) ?? state])), "manifest");
  }

  /** One file's state, as an update says. */
  put(path: string, state: FileState) {
    this.write(new Map([[path, state]]), "update");
  }

  /** Drops every file and what is known of their order. */
  reset() {
    this.cells.clear();
    this.held = new ReviewOrder();
  }

  /** Every write of review state goes through here: the cells take it and the listeners hear which paths changed. */
  private write(entries: ReadonlyMap<string, FileState>, cause: "update" | "manifest") {
    if (cause === "manifest") this.cells.clear();
    const stamp = ++this.clock;
    for (const [path, file] of entries) {
      this.cells.set(path, file);
      this.versions.set(path, stamp);
    }
    const paths = cause === "manifest" ? "all" : new Set(entries.keys());
    for (const listener of this.listeners) listener(paths);
  }

  /** Tells the listeners that every file was replaced by nothing: what the view showed is gone. */
  clearedAll() {
    this.write(new Map(), "manifest");
  }

  subscribe(listener: WriteListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}

export type ReviewCells = Cells;

export const newReviewCells = (): ReviewCells => new Cells();
