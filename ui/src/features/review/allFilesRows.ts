// The rows of the "All files" tree, from the directory listings loaded so far. Like the Changes
// tree, a directory whose only child is a directory shows as one row (`crates/vet-core/src`);
// here that is known only once the listing of the child is loaded.

import type { DirEntry } from "../../generated/DirEntry";
import { compactChain, joinPath } from "../../lib/paths.ts";

export interface AllRow {
  kind: "dir" | "file";
  /** A directory's first path of its compacted chain, which expansion is keyed by; a file's path. */
  key: string;
  /** A directory's last path of the chain, whose listing is shown below it. */
  path: string;
  name: string;
  depth: number;
  entry: DirEntry;
}

/** The name of the directory that is all of `listing`, if it is one. */
export const onlyDirectory = (listing: readonly DirEntry[] | undefined): string | undefined =>
  listing?.length === 1 && listing[0].kind === "dir" ? listing[0].name : undefined;

/** The visible rows; the contents of directories not in `expanded`, or not loaded yet, are left out. */
export function buildAllRows(
  listings: ReadonlyMap<string, readonly DirEntry[]>,
  expanded: ReadonlySet<string>,
): AllRow[] {
  const rows: AllRow[] = [];
  const onlyChild = (path: string) => {
    const name = onlyDirectory(listings.get(path));
    return name === undefined ? undefined : joinPath(path, name);
  };
  const walk = (dir: string, depth: number) => {
    for (const entry of listings.get(dir) ?? []) {
      const key = joinPath(dir, entry.name);
      if (entry.kind !== "dir") {
        rows.push({ kind: "file", key, path: key, name: entry.name, depth, entry });
        continue;
      }
      const { path, name } = compactChain(key, entry.name, onlyChild);
      rows.push({ kind: "dir", key, path, name, depth, entry });
      if (expanded.has(key)) walk(path, depth + 1);
    }
  };
  walk("", 0);
  return rows;
}
