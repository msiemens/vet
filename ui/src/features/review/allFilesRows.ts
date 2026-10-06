// The rows of the "All files" tree, from the directory listings loaded so far. Like the Changes
// tree, a directory whose only child is a directory shows as one row (`crates/vet-core/src`);
// here that is known only once the listing of the child is loaded.

import type { DirEntry } from "../../generated/DirEntry";

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

export const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

/** The visible rows; the contents of directories not in `expanded`, or not loaded yet, are left out. */
export function buildAllRows(
  listings: ReadonlyMap<string, readonly DirEntry[]>,
  expanded: ReadonlySet<string>,
): AllRow[] {
  const rows: AllRow[] = [];
  const walk = (dir: string, depth: number) => {
    for (const entry of listings.get(dir) ?? []) {
      const key = joinPath(dir, entry.name);
      if (entry.kind !== "dir") {
        rows.push({ kind: "file", key, path: key, name: entry.name, depth, entry });
        continue;
      }
      let [path, name] = [key, entry.name];
      for (;;) {
        const only = listings.get(path);
        if (only?.length !== 1 || only[0].kind !== "dir") break;
        path = joinPath(path, only[0].name);
        name += `/${only[0].name}`;
      }
      rows.push({ kind: "dir", key, path, name, depth, entry });
      if (expanded.has(key)) walk(path, depth + 1);
    }
  };
  walk("", 0);
  return rows;
}
