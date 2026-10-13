// The rows of the repository's folder tree: the folders listed so far, nested under the ones that are open.

import { joinPath } from "../../lib/paths.ts";

export interface FolderRow {
  path: string;
  name: string;
  depth: number;
  /** Whether it shows its folders; none once it was listed and holds no folder. */
  open: boolean | undefined;
}

/** `listed` maps a folder (`""` is the root) to the names of the folders directly in it, for each folder listed so far. */
export function folderRows(listed: ReadonlyMap<string, readonly string[]>, open: ReadonlySet<string>): FolderRow[] {
  const rows: FolderRow[] = [];
  const walk = (parent: string, depth: number) => {
    for (const name of listed.get(parent) ?? []) {
      const path = joinPath(parent, name);
      const children = listed.get(path);
      const isOpen = open.has(path);
      rows.push({ path, name, depth, open: children?.length === 0 ? undefined : isOpen });
      if (isOpen) walk(path, depth + 1);
    }
  };
  walk("", 0);
  return rows;
}
