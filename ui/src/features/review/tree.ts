// The changed-files tree as a flat list of rows, grouped by directory.

import type { FileChange } from "../../generated/FileChange";
import type { FileStatus } from "../../generated/FileStatus";
import { coverage } from "../../state/changeset.svelte";
import { mockIsBuildFile } from "./mock";

export interface DirRow {
  kind: "dir";
  path: string;
}

export interface FileRow {
  kind: "file";
  path: string;
  name: string;
  file: FileChange;
  status: "M" | "A" | "D" | "R";
  seenPct: number;
  oldPct: number;
  build: boolean;
  /** The mode changed. */
  executable: boolean;
}

export type Row = DirRow | FileRow;

const LETTER: Record<FileStatus, FileRow["status"]> = { modified: "M", added: "A", deleted: "D", renamed: "R" };

const dirname = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

export function buildRows(files: FileChange[], collapsed: ReadonlySet<string>): Row[] {
  const items: FileRow[] = files.map((file) => {
    const c = coverage(file);
    return {
      kind: "file",
      path: file.path,
      name: file.path.slice(file.path.lastIndexOf("/") + 1),
      file,
      status: LETTER[file.status],
      seenPct: c.total ? (c.seen / c.total) * 100 : 0,
      oldPct: c.total ? (c.old / c.total) * 100 : 0,
      build: mockIsBuildFile(file.path),
      executable: file.nonText.includes("mode"),
    };
  });
  items.sort((a, b) => dirname(a.path).localeCompare(dirname(b.path)) || a.path.localeCompare(b.path));
  const rows: Row[] = [];
  let dir: string | undefined;
  for (const item of items) {
    const d = dirname(item.path);
    if (d !== dir) {
      dir = d;
      if (d) rows.push({ kind: "dir", path: d });
    }
    if (!d || !collapsed.has(d)) rows.push(item);
  }
  return rows;
}
