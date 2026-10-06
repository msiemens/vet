// The changed-files tree. Chains of directories with a single directory child and no files
// are shown as one row (`crates/vet-core/src`).

import type { FileChange } from "../../generated/FileChange";
import type { FileStatus } from "../../generated/FileStatus";
import { mockIsBuildFile } from "./mock.ts";

export interface DirRow {
  kind: "dir";
  /** Of the last directory of a compacted chain. */
  path: string;
  /** The compacted chain, e.g. `crates/vet-core/src`. */
  name: string;
  depth: number;
  /** The paths of the directory rows below this one. */
  descendants: string[];
}

export interface FileRow {
  kind: "file";
  path: string;
  name: string;
  depth: number;
  file: FileChange;
  status: "M" | "A" | "D" | "R";
  seenPct: number;
  oldPct: number;
  build: boolean;
  /** Set when the mode changed: the executable bit gained or lost, else a plain mode change. */
  mode?: "+x" | "-x" | "mode";
}

export type Row = DirRow | FileRow;

interface DirNode {
  path: string;
  name: string;
  dirs: DirNode[];
  files: FileChange[];
}

const LETTER: Record<FileStatus, FileRow["status"]> = { modified: "M", added: "A", deleted: "D", renamed: "R" };

function modeChange(file: FileChange): FileRow["mode"] {
  const item = file.nonText.find((i) => i.kind === "mode");
  if (!item) return undefined;
  const [before, after] = [(item.before?.mode ?? 0) & 0o111, (item.after?.mode ?? 0) & 0o111];
  return after && !before ? "+x" : before && !after ? "-x" : "mode";
}

const basename = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const byName = (a: string, b: string) => a.localeCompare(b);

/** Directories first, then files, each by name; single-directory chains merged. */
export function buildTree(files: FileChange[]): DirNode {
  const root: DirNode = { path: "", name: "", dirs: [], files: [] };
  const dirs = new Map<string, DirNode>([["", root]]);
  const dirOf = (path: string): DirNode => {
    let dir = dirs.get(path);
    if (!dir) {
      dir = { path, name: basename(path), dirs: [], files: [] };
      dirs.set(path, dir);
      dirOf(path.slice(0, Math.max(0, path.lastIndexOf("/")))).dirs.push(dir);
    }
    return dir;
  };
  for (const file of files) dirOf(file.path.slice(0, Math.max(0, file.path.lastIndexOf("/")))).files.push(file);

  const compact = (dir: DirNode): DirNode => {
    dir.dirs = dir.dirs.map(compact).sort((a, b) => byName(a.name, b.name));
    dir.files.sort((a, b) => byName(basename(a.path), basename(b.path)));
    const [only] = dir.dirs;
    return dir.dirs.length === 1 && !dir.files.length && dir.path ? { ...only, name: `${dir.name}/${only.name}` } : dir;
  };
  root.dirs = root.dirs.map(compact).sort((a, b) => byName(a.name, b.name));
  root.files.sort((a, b) => byName(basename(a.path), basename(b.path)));
  return root;
}

function fileRow(file: FileChange, depth: number): FileRow {
  const c = file.completion;
  const total = c.lines + c.items;
  return {
    kind: "file",
    path: file.path,
    name: basename(file.path),
    depth,
    file,
    status: LETTER[file.status],
    seenPct: total ? ((c.seenLines + c.acknowledgedItems) / total) * 100 : 0,
    oldPct: total ? (c.oldBaseLines / total) * 100 : 0,
    build: mockIsBuildFile(file.path),
    mode: modeChange(file),
  };
}

const dirPaths = (dir: DirNode): string[] => dir.dirs.flatMap((d) => [d.path, ...dirPaths(d)]);

/** The visible rows; the contents of `collapsed` directories are left out. */
export function buildRows(root: DirNode, collapsed: ReadonlySet<string>): Row[] {
  const rows: Row[] = [];
  const walk = (dir: DirNode, depth: number) => {
    for (const d of dir.dirs) {
      rows.push({ kind: "dir", path: d.path, name: d.name, depth, descendants: dirPaths(d) });
      if (!collapsed.has(d.path)) walk(d, depth + 1);
    }
    for (const file of dir.files) rows.push(fileRow(file, depth));
  };
  walk(root, 0);
  return rows;
}
