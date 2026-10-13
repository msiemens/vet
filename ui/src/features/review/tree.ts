// The changed-files tree. Its structure depends on the paths alone, so a review update
// (which changes a file's progress, not its path) never rebuilds it. Chains of
// directories with a single directory child and no files are shown as one row
// (`crates/vet-core/src`).

import type { FileStatus } from "../../generated/FileStatus";
import type { ModeChange } from "../../generated/ModeChange";
import { basename, compactChain, dirname } from "../../lib/paths.ts";
import { match, type Range } from "../../lib/speedSearch.ts";
import type { FileLayout, ReviewedFile } from "../../state/reviewModel.ts";
import type { SymbolRow } from "./symbolRows.ts";

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

export type Row = DirRow | FileRow | SymbolRow;

/** What a file row shows besides its name: it changes with review updates. */
export interface FileSummary {
  status: "M" | "A" | "D" | "R";
  seenPercent: number;
  oldBasePercent: number;
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

const MODE: Record<ModeChange, NonNullable<FileSummary["mode"]>> = {
  gainedExecutable: "+x",
  lostExecutable: "-x",
  other: "mode",
};

/** The mode change a file shows next to its name, if its mode changed. */
export function modeChange(file: FileLayout): FileSummary["mode"] {
  const change = file.nonText.find((i) => i.kind === "mode")?.modeChange;
  return change ? MODE[change] : undefined;
}

/** The letter that stands for how a file changed. */
export const statusLetter = (file: FileLayout): FileSummary["status"] => LETTER[file.status];

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

  const onlyChild = (path: string) => {
    const { dirs: below, files } = dirs.get(path) ?? root;
    return below.length === 1 && !files.length ? below[0].path : undefined;
  };
  const finish = (node: DirNode): DirNode => {
    const { path, name } = node.path ? compactChain(node.path, node.name, onlyChild) : node;
    const last = dirs.get(path) ?? root;
    const below = last.dirs.map(finish).sort((a, b) => byName(a.name, b.name));
    const files = last.files.sort((a, b) => byName(basename(a), basename(b)));
    return { path, name, dirs: below, files, below: below.flatMap((d) => [d.path, ...d.below]) };
  };
  return finish(root);
}

export function summarize(file: ReviewedFile): FileSummary {
  const { seenPercent, oldBasePercent } = file.completion;
  return { status: statusLetter(file), seenPercent, oldBasePercent, mode: modeChange(file) };
}

/**
 * The rows of a speed search over `rows` (a tree in display order, its folders open): those whose
 * name matches `query`, and the rows that hold them (folders, files with symbols, symbols), whatever
 * their names. `hits` has the matches' ranges to highlight.
 */
export function filterRows<R extends { depth: number; name: string }>(
  rows: readonly R[],
  query: string,
): { rows: R[]; hits: Map<R, Range[]> } {
  const hits = new Map<R, Range[]>();
  const kept: R[] = [];
  // below[d]: a row at depth d is kept in the subtree being walked back through.
  const below: (boolean | undefined)[] = [];
  for (let i = rows.length - 1; i >= 0; i--) {
    const row = rows[i];
    const found = match(query, row.name);
    if (found) hits.set(row, found.ranges);
    const keep = !!found || !!below[row.depth + 1];
    below.length = row.depth + 1;
    below[row.depth] = keep || !!below[row.depth];
    if (keep) kept.push(row);
  }
  return { rows: kept.reverse(), hits };
}

/**
 * The visible rows; the contents of `collapsed` directories are left out. `symbols` gives the rows
 * of a file's symbols, from the depth they start at, to show below its row.
 */
export function buildRows(
  root: DirNode,
  collapsed: ReadonlySet<string>,
  symbols?: (path: string, depth: number) => SymbolRow[],
): Row[] {
  const rows: Row[] = [];
  const walk = (dir: DirNode, depth: number) => {
    for (const d of dir.dirs) {
      rows.push({ kind: "dir", path: d.path, name: d.name, depth, descendants: d.below });
      if (!collapsed.has(d.path)) walk(d, depth + 1);
    }
    for (const path of dir.files) {
      rows.push({ kind: "file", path, name: basename(path), depth });
      if (symbols) rows.push(...symbols(path, depth + 1));
    }
  };
  walk(root, 0);
  return rows;
}
