// The changed-files tree as a flat list of rows, grouped by directory.

import type { FileChange } from "../../generated/FileChange";
import type { FileStatus } from "../../generated/FileStatus";
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
  /** Set when the mode changed: the executable bit gained or lost, else a plain mode change. */
  mode?: "+x" | "-x" | "mode";
}

export type Row = DirRow | FileRow;

const LETTER: Record<FileStatus, FileRow["status"]> = { modified: "M", added: "A", deleted: "D", renamed: "R" };

function modeChange(file: FileChange): FileRow["mode"] {
  const item = file.nonText.find((i) => i.kind === "mode");
  if (!item) return undefined;
  const [before, after] = [(item.before?.mode ?? 0) & 0o111, (item.after?.mode ?? 0) & 0o111];
  return after && !before ? "+x" : before && !after ? "-x" : "mode";
}

const dirname = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

export function buildRows(files: FileChange[], collapsed: ReadonlySet<string>): Row[] {
  const items: FileRow[] = files.map((file) => {
    const c = file.completion;
    const total = c.lines + c.items;
    return {
      kind: "file",
      path: file.path,
      name: file.path.slice(file.path.lastIndexOf("/") + 1),
      file,
      status: LETTER[file.status],
      seenPct: total ? ((c.seenLines + c.acknowledgedItems) / total) * 100 : 0,
      oldPct: total ? (c.oldBaseLines / total) * 100 : 0,
      build: mockIsBuildFile(file.path),
      mode: modeChange(file),
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
