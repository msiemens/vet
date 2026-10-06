// The changed-files tree. Its structure depends on the paths alone, so a review update
// (which changes a file's progress, not its path) never rebuilds it. Chains of
// directories with a single directory child and no files are shown as one row
// (`crates/vet-core/src`).

import type { FileChange } from "../../generated/FileChange";
import type { FileStatus } from "../../generated/FileStatus";

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
}

export type Row = DirRow | FileRow;

/** What a file row shows besides its name: it changes with review updates. */
export interface FileSummary {
  status: "M" | "A" | "D" | "R";
  seenPct: number;
  oldPct: number;
  /** Set when the mode changed: the executable bit gained or lost, else a plain mode change. */
  mode?: "+x" | "-x" | "mode";
}

interface DirNode {
  path: string;
  name: string;
  dirs: DirNode[];
  files: string[];
  /** Paths of the directories below, in tree order. */
  below: string[];
}

const LETTER: Record<FileStatus, FileSummary["status"]> = { modified: "M", added: "A", deleted: "D", renamed: "R" };

function modeChange(file: FileChange): FileSummary["mode"] {
  const item = file.nonText.find((i) => i.kind === "mode");
  if (!item) return undefined;
  const [before, after] = [(item.before?.mode ?? 0) & 0o111, (item.after?.mode ?? 0) & 0o111];
  return after && !before ? "+x" : before && !after ? "-x" : "mode";
}

const basename = (path: string) => path.slice(path.lastIndexOf("/") + 1);
const dirname = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));
// One collator: `localeCompare` creates one per call.
const collator = new Intl.Collator();
const byName = (a: string, b: string) => collator.compare(a, b);

/** Directories first, then files, each by name; single-directory chains merged. */
export function buildTree(paths: readonly string[]): DirNode {
  const root: DirNode = { path: "", name: "", dirs: [], files: [], below: [] };
  const dirs = new Map<string, DirNode>([["", root]]);
  const dirOf = (path: string): DirNode => {
    let dir = dirs.get(path);
    if (!dir) {
      dir = { path, name: basename(path), dirs: [], files: [], below: [] };
      dirs.set(path, dir);
      dirOf(dirname(path)).dirs.push(dir);
    }
    return dir;
  };
  for (const path of paths) dirOf(dirname(path)).files.push(path);

  const finish = (dir: DirNode): DirNode => {
    dir.dirs = dir.dirs.map(finish).sort((a, b) => byName(a.name, b.name));
    dir.files.sort((a, b) => byName(basename(a), basename(b)));
    dir.below = dir.dirs.flatMap((d) => [d.path, ...d.below]);
    const [only] = dir.dirs;
    return dir.dirs.length === 1 && !dir.files.length && dir.path ? { ...only, name: `${dir.name}/${only.name}` } : dir;
  };
  return finish(root);
}

export function summarize(file: FileChange): FileSummary {
  const c = file.completion;
  const total = c.lines + c.items;
  return {
    status: LETTER[file.status],
    seenPct: total ? ((c.seenLines + c.acknowledgedItems) / total) * 100 : 0,
    oldPct: total ? (c.oldBaseLines / total) * 100 : 0,
    mode: modeChange(file),
  };
}

/** The visible rows; the contents of `collapsed` directories are left out. */
export function buildRows(root: DirNode, collapsed: ReadonlySet<string>): Row[] {
  const rows: Row[] = [];
  const walk = (dir: DirNode, depth: number) => {
    for (const d of dir.dirs) {
      rows.push({ kind: "dir", path: d.path, name: d.name, depth, descendants: d.below });
      if (!collapsed.has(d.path)) walk(d, depth + 1);
    }
    for (const path of dir.files) rows.push({ kind: "file", path, name: basename(path), depth });
  };
  walk(root, 0);
  return rows;
}
