// The changed-files tree as a flat list of rows, grouped by directory.

import { type FileState, lineCoverage } from "../../state/changeset.svelte";
import type { NonTextChange } from "./ChangeCard.svelte";
import type { HunkState } from "./editor/hunkBar";
import { mockIsBuildFile, mockIsExecutableChange } from "./mock";

export interface DirRow {
  kind: "dir";
  path: string;
}

export interface FileRow {
  kind: "file";
  path: string;
  name: string;
  /** Index into the text files, for text changes. */
  index?: number;
  nonText?: NonTextChange;
  status: "M" | "A" | "D";
  seenPct: number;
  oldPct: number;
  build: boolean;
  executable: boolean;
}

export type Row = DirRow | FileRow;

const dirname = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

export function buildRows(files: FileState[], nonText: { change: NonTextChange; state: HunkState }[], collapsed: ReadonlySet<string>): Row[] {
  const items: FileRow[] = [];
  files.forEach((f, index) => {
    const c = lineCoverage(f);
    const { path, baseLines, targetLines } = f.entry;
    items.push({
      kind: "file",
      path,
      name: path.slice(path.lastIndexOf("/") + 1),
      index,
      status: baseLines === 0 ? "A" : targetLines === 0 ? "D" : "M",
      seenPct: c.total ? (c.seen / c.total) * 100 : 0,
      oldPct: c.total ? (c.old / c.total) * 100 : 0,
      build: mockIsBuildFile(path),
      executable: mockIsExecutableChange(path),
    });
  });
  for (const { change, state } of nonText) {
    items.push({
      kind: "file",
      path: change.path,
      name: change.path.slice(change.path.lastIndexOf("/") + 1),
      nonText: change,
      status: "M",
      seenPct: state === "seen" ? 100 : 0,
      oldPct: state === "oldBase" ? 100 : 0,
      build: mockIsBuildFile(change.path),
      executable: change.kind === "mode",
    });
  }
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
