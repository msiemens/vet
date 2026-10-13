// The Problems panel's logic, free of components: a changeset's diagnostics as problems that know
// the hunk they sit in, the filters over them and the rows of the list they make. The backend says
// what a diagnostic is and whether the change introduced it; this orders and groups.

import type { Diagnostic } from "../generated/Diagnostic";
import type { DiagnosticOrigin } from "../generated/DiagnosticOrigin";
import type { Hunk } from "../generated/Hunk";
import type { HunkState } from "../generated/HunkState";

/** The review state of the hunk a problem sits in; `none` for a line the change did not touch or sit near. */
export type Reviewed = "seen" | "old" | "unseen" | "none";

/** The three severities the panel tells apart: hints are information. */
export type Level = "error" | "warning" | "info";

export type Grouping = "origin" | "file" | "severity";

/** How many unchanged lines around a hunk still count as in it, as the editors show them. */
export const CONTEXT = 3;

export interface Problem {
  /** Stable while the file's diagnostics stay the same. */
  id: string;
  path: string;
  /** 0-based line of the start of the diagnostic. */
  line: number;
  level: Level;
  origin: DiagnosticOrigin;
  diagnostic: Diagnostic;
  reviewed: Reviewed;
  /** The line is one the change added or changed. */
  onChanged: boolean;
}

/** A changed file's hunks and what has been seen of them. */
export interface FileHunks {
  hunks: readonly Hunk[];
  states: readonly HunkState[];
}

export const levelOf = (d: Diagnostic): Level => (d.severity === "hint" ? "info" : d.severity);

const REVIEWED: Record<HunkState, Reviewed> = {
  unseen: "unseen",
  seen: "seen",
  seenAgainstOldBase: "old",
  informational: "none",
  unapplied: "none",
};

/** Where the 0-based target `line` stands against the file's hunks. */
export function reviewedAt(file: FileHunks | undefined, line: number): { reviewed: Reviewed; onChanged: boolean } {
  const outside = { reviewed: "none", onChanged: false } as const;
  if (!file) return outside;
  const at = file.hunks.findIndex(
    (h) => line >= h.targetStart - CONTEXT && line < h.targetStart + h.targetLen + CONTEXT,
  );
  if (at < 0) return outside;
  const h = file.hunks[at];
  return {
    reviewed: REVIEWED[file.states[at] ?? "unseen"],
    onChanged: line >= h.targetStart && line < h.targetStart + h.targetLen,
  };
}

/** A file's diagnostics as problems, in the order of the file. */
export function problemsOf(path: string, diagnostics: readonly Diagnostic[], file: FileHunks | undefined): Problem[] {
  return diagnostics
    .map((diagnostic) => {
      const { line, col } = diagnostic.span.start;
      return {
        id: `${path}:${line}:${col}:${diagnostic.code ?? ""}:${diagnostic.message}`,
        path,
        line,
        level: levelOf(diagnostic),
        origin: diagnostic.origin,
        diagnostic,
        ...reviewedAt(file, line),
      };
    })
    .sort((a, b) => a.line - b.line || a.diagnostic.span.start.col - b.diagnostic.span.start.col);
}

export interface Filters {
  levels: Record<Level, boolean>;
  /** `new` lists only what the change introduced; it has no effect while origins are not known. */
  origin: "new" | "all";
  /** Only problems in hunks not seen. */
  unseenOnly: boolean;
  /** Only problems on added or changed lines. */
  changedOnly: boolean;
  query: string;
}

export const NO_FILTERS: Filters = {
  levels: { error: true, warning: true, info: true },
  origin: "all",
  unseenOnly: false,
  changedOnly: false,
  query: "",
};

/** Whether any filter narrows the list. */
export const narrowing = (f: Filters) =>
  !f.levels.error ||
  !f.levels.warning ||
  !f.levels.info ||
  f.origin === "new" ||
  f.unseenOnly ||
  f.changedOnly ||
  f.query.trim() !== "";

/** The problems that pass the filters, in the order they came. `known` says whether origins are known. */
export function filtered(problems: readonly Problem[], f: Filters, known: boolean): Problem[] {
  const query = f.query.trim().toLowerCase();
  return problems.filter((p) => {
    if (!f.levels[p.level]) return false;
    if (f.origin === "new" && known && p.origin !== "new") return false;
    if (f.unseenOnly && p.reviewed !== "unseen") return false;
    if (f.changedOnly && !p.onChanged) return false;
    if (query) {
      const text = `${p.diagnostic.message} ${p.diagnostic.source ?? ""} ${p.diagnostic.code ?? ""} ${p.path}`;
      if (!text.toLowerCase().includes(query)) return false;
    }
    return true;
  });
}

const byPlace = (a: Problem, b: Problem) => (a.path < b.path ? -1 : a.path > b.path ? 1 : a.line - b.line);

/** New problems before the rest, then by file and line: the order of stepping through them. */
export function inOrder(problems: readonly Problem[]): Problem[] {
  const rank = (p: Problem) => (p.origin === "new" ? 0 : 1);
  return [...problems].sort((a, b) => rank(a) - rank(b) || byPlace(a, b));
}

/** The problem `delta` after the one with id `from` in `list`, wrapping around; the first (last) one with none selected. */
export function step(list: readonly Problem[], from: string | undefined, delta: 1 | -1): Problem | undefined {
  if (!list.length) return undefined;
  const at = list.findIndex((p) => p.id === from);
  if (at < 0) return delta > 0 ? list[0] : list[list.length - 1];
  return list[(at + delta + list.length) % list.length];
}

export type Row =
  | { kind: "group"; key: string; title: string; count: number; open: boolean; tone: "new" | "plain" | Level }
  | { kind: "file"; key: string; path: string; count: number; open: boolean }
  | { kind: "problem"; key: string; problem: Problem; showFile: boolean; pill: boolean };

export interface Layout {
  grouping: Grouping;
  /** Every problem's origin is known, so the list may be sectioned by it and rows need no origin pill. */
  known: boolean;
  /** The keys of the groups and files that are folded. */
  folded: ReadonlySet<string>;
}

const LEVELS: Level[] = ["error", "warning", "info"];
const LEVEL_TITLE: Record<Level, string> = { error: "Errors", warning: "Warnings", info: "Information and hints" };

function filesOf(problems: readonly Problem[]): string[] {
  return [...new Set(problems.map((p) => p.path))].sort();
}

function fileRows(problems: readonly Problem[], keyPrefix: string, layout: Layout, pill: boolean): Row[] {
  const rows: Row[] = [];
  for (const path of filesOf(problems)) {
    const own = problems.filter((p) => p.path === path).sort(byPlace);
    const key = `${keyPrefix}${path}`;
    const open = !layout.folded.has(key);
    rows.push({ kind: "file", key, path, count: own.length, open });
    if (open) for (const problem of own) rows.push({ kind: "problem", key, problem, showFile: false, pill });
  }
  return rows;
}

/** The rows of the list: by origin and file (when origins are known), by file, or by severity. */
export function rowsOf(problems: readonly Problem[], layout: Layout): Row[] {
  const { grouping, known, folded } = layout;
  if (grouping === "origin" && known) {
    const rows: Row[] = [];
    for (const [origin, title] of [
      ["new", "New in this change"],
      ["existing", "Already there"],
    ] as const) {
      const own = problems.filter((p) => p.origin === origin);
      if (!own.length) continue;
      const key = `o:${origin}`;
      const open = !folded.has(key);
      rows.push({ kind: "group", key, title, count: own.length, open, tone: origin === "new" ? "new" : "plain" });
      if (open) rows.push(...fileRows(own, `${key}:`, layout, false));
    }
    return rows;
  }
  if (grouping === "severity") {
    const rows: Row[] = [];
    for (const level of LEVELS) {
      const own = problems
        .filter((p) => p.level === level)
        .sort((a, b) => (a.origin === "new" ? 0 : 1) - (b.origin === "new" ? 0 : 1) || byPlace(a, b));
      if (!own.length) continue;
      const key = `s:${level}`;
      const open = !folded.has(key);
      rows.push({ kind: "group", key, title: LEVEL_TITLE[level], count: own.length, open, tone: level });
      if (open) for (const problem of own) rows.push({ kind: "problem", key, problem, showFile: true, pill: true });
    }
    return rows;
  }
  return fileRows(problems, "f:", layout, !known || grouping === "file");
}

/** What the footer and the rail say of a changeset's problems. */
export interface Counts {
  /** New errors and warnings; with origins not known, all errors and warnings. */
  headline: number;
  /** The worst severity among those counted. */
  worst?: "error" | "warning";
  /** Errors, warnings and information together. */
  all: number;
  newOnes: number;
  existing: number;
}

/** `known` says every origin is known: only the new problems are counted then. */
export function countsOf(problems: readonly Problem[], known: boolean): Counts {
  const counted = problems.filter((p) => p.level !== "info" && (!known || p.origin === "new"));
  return {
    headline: counted.length,
    worst: counted.some((p) => p.level === "error") ? "error" : counted.length ? "warning" : undefined,
    all: problems.length,
    newOnes: problems.filter((p) => p.origin === "new").length,
    existing: problems.filter((p) => p.origin === "existing").length,
  };
}
