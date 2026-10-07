// The "All files" tree's state: the directories listed so far and which are expanded. Directories
// load lazily when expanded. Listings belong to a manifest revision; a reply of another revision
// is dropped, and a new revision reloads the root and every expanded directory, keeping what is
// shown until the new listing replaces it.

import { SvelteMap, SvelteSet } from "svelte/reactivity";
import type { DirEntry } from "../../generated/DirEntry";
import { joinPath } from "../../lib/paths";
import { type AllRow, buildAllRows, onlyDirectory } from "./allFilesRows";

type List = (path: string) => Promise<{ revision: number; entries: DirEntry[] } | undefined>;

export class AllFiles {
  private listings = new SvelteMap<string, DirEntry[]>();
  readonly expanded = new SvelteSet<string>();
  private revision?: number;
  rows: AllRow[] = $derived(buildAllRows(this.listings, this.expanded));

  constructor(private list: List) {}

  /** Follows the manifest to `revision`. */
  reset(revision: number) {
    this.revision = revision;
    for (const path of ["", ...this.expanded]) void this.load(path);
  }

  /** Expands or collapses a directory row by its key. */
  toggle(key: string) {
    if (this.expanded.delete(key)) return;
    this.expanded.add(key);
    void this.load(key);
  }

  /** Lists `path`; a directory whose only child is a directory also lists that, so the chain can show as one row. */
  private async load(path: string): Promise<void> {
    const revision = this.revision;
    const reply = await this.list(path);
    if (!reply || reply.revision !== revision) return;
    this.listings.set(path, reply.entries);
    const only = onlyDirectory(reply.entries);
    if (path !== "" && only !== undefined) await this.load(joinPath(path, only));
  }
}
