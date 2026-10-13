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
  const seen = new Map<string, number>();
  return diagnostics
    .map((diagnostic) => {
      const { line, col } = diagnostic.span.start;
      const same = [
        path,
        line,
        col,
        diagnostic.severity,
        diagnostic.source ?? "",
        diagnostic.code ?? "",
        diagnostic.message,
      ];
      const key = same.join(":");
      const nth = seen.get(key) ?? 0;
      seen.set(key, nth + 1);
      return {
        id: `${key}#${nth}`,
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
  /** `new` lists only what the change introduced; it has no effect while an origin is pending. */
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

/** The problems that pass the filters, in the order they came. `settled` says no origin is pending. */
export function filtered(problems: readonly Problem[], f: Filters, settled: boolean): Problem[] {
  const query = f.query.trim().toLowerCase();
  return problems.filter((p) => {
    if (!f.levels[p.level]) return false;
    if (f.origin === "new" && settled && p.origin !== "new") return false;
    if (f.unseenOnly && p.reviewed !== "unseen") return false;
    if (f.changedOnly && !p.onChanged) return false;
    if (query) {
      const text = `${p.diagnostic.message} ${p.diagnostic.source ?? ""} ${p.diagnostic.code ?? ""} ${p.path}`;
      if (!text.toLowerCase().includes(query)) return false;
    }
    return true;
  });
}

/** Why the list shows no problem. */
export type Empty =
  /** The filters, the origin switch included, hide `hidden` problems of the scope, and only clearing them shows any. */
  | { kind: "hidden"; hidden: number }
  /** Only the New switch hides the scope's problems, all of which the base has too. */
  | { kind: "onlyExisting"; existing: number }
  /** Some file of the scope has not answered, so none is not known yet. */
  | { kind: "partial" }
  | { kind: "none" };

/** What to say where `shown` is empty; nothing where the list has problems. */
export function emptyOf(input: {
  all: readonly Problem[];
  shown: readonly Problem[];
  filters: Filters;
  settled: boolean;
  /** Files of the scope still to be answered. */
  unanswered: number;
}): Empty | undefined {
  const { all, shown, filters, settled, unanswered } = input;
  if (shown.length) return undefined;
  if (!all.length) return unanswered ? { kind: "partial" } : { kind: "none" };
  const onlyOrigin =
    filters.origin === "new" &&
    settled &&
    !narrowing({ ...filters, origin: "all" }) &&
    all.every((p) => p.origin === "existing");
  return onlyOrigin ? { kind: "onlyExisting", existing: all.length } : { kind: "hidden", hidden: all.length };
}

const byPlace = (a: Problem, b: Problem) => (a.path < b.path ? -1 : a.path > b.path ? 1 : a.line - b.line);

/**
 * The problem after (`delta` 1) or before the caret among `list`, one file's problems in file
 * order, wrapping around within the file. `selected` is the one last stepped to: while the caret
 * is still on its line the step goes on from it, so problems on one line are each reached.
 */
export function stepInFile(
  list: readonly Problem[],
  caretLine: number | undefined,
  selected: string | undefined,
  delta: 1 | -1,
): Problem | undefined {
  const n = list.length;
  if (!n) return undefined;
  const at = list.findIndex((p) => p.id === selected);
  if (at >= 0 && list[at].line === caretLine) return list[(at + delta + n) % n];
  if (caretLine === undefined) return delta > 0 ? list[0] : list[n - 1];
  if (delta > 0) return list.find((p) => p.line > caretLine) ?? list[0];
  return [...list].reverse().find((p) => p.line < caretLine) ?? list[n - 1];
}

/** A row of the list; `depth` is how far it sits inside the groups and files above it. */
export type Row =
  | {
      kind: "group";
      depth: number;
      key: string;
      title: string;
      count: number;
      open: boolean;
      tone: "new" | "plain" | Level;
    }
  | { kind: "file"; depth: number; key: string; path: string; count: number; open: boolean }
  | { kind: "problem"; depth: number; key: string; problem: Problem; showFile: boolean; pill: boolean };

export interface Layout {
  grouping: Grouping;
  /** No origin is pending, so the list may be sectioned by it and rows need no origin pill. */
  settled: boolean;
  /** The keys of the groups and files that are folded. */
  folded: ReadonlySet<string>;
}

const LEVELS: Level[] = ["error", "warning", "info"];
const LEVEL_TITLE: Record<Level, string> = { error: "Errors", warning: "Warnings", info: "Information and hints" };

/** The problems by file, the files in path order. */
function byFile(problems: readonly Problem[]): [string, Problem[]][] {
  const files = new Map<string, Problem[]>();
  for (const p of problems) {
    const own = files.get(p.path);
    if (own) own.push(p);
    else files.set(p.path, [p]);
  }
  return [...files].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

function fileRows(
  problems: readonly Problem[],
  keyPrefix: string,
  layout: Layout,
  pill: boolean,
  depth: number,
): Row[] {
  const rows: Row[] = [];
  for (const [path, found] of byFile(problems)) {
    const own = found.sort(byPlace);
    const key = `${keyPrefix}${path}`;
    const open = !layout.folded.has(key);
    rows.push({ kind: "file", depth, key, path, count: own.length, open });
    if (open)
      for (const problem of own) rows.push({ kind: "problem", depth: depth + 1, key, problem, showFile: false, pill });
  }
  return rows;
}

/** The rows of the list: by origin and file (once no origin is pending), by file, or by severity. */
export function rowsOf(problems: readonly Problem[], layout: Layout): Row[] {
  const { grouping, folded } = layout;
  // A problem still being compared has no origin group to sit in, so it is never left out.
  const settled = layout.settled && !problems.some((p) => p.origin === "pending");
  if (grouping === "origin" && settled) {
    const rows: Row[] = [];
    for (const [origin, title] of [
      ["new", "New in this change"],
      ["existing", "Already there"],
      ["unknown", "Origin unknown"],
    ] as const) {
      const own = problems.filter((p) => p.origin === origin);
      if (!own.length) continue;
      const key = `o:${origin}`;
      const open = !folded.has(key);
      rows.push({
        kind: "group",
        depth: 0,
        key,
        title,
        count: own.length,
        open,
        tone: origin === "new" ? "new" : "plain",
      });
      if (open) rows.push(...fileRows(own, `${key}:`, layout, false, 1));
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
      rows.push({ kind: "group", depth: 0, key, title: LEVEL_TITLE[level], count: own.length, open, tone: level });
      if (open)
        for (const problem of own) rows.push({ kind: "problem", depth: 1, key, problem, showFile: true, pill: true });
    }
    return rows;
  }
  return fileRows(problems, "f:", layout, !settled || grouping === "file", 0);
}

/** What the footer and the rail say of a changeset's problems. */
export interface Counts {
  /** Errors and warnings known to be new, and those still being compared (they may be); not those of unknown origin. */
  headline: number;
  /** The worst severity among those counted. */
  worst?: "error" | "warning";
  /** Errors, warnings and information together. */
  all: number;
  newOnes: number;
  existing: number;
  /** Problems whose origin could not be told. */
  unknown: number;
  /** Information and hints, of any origin. */
  info: number;
}

export function countsOf(problems: readonly Problem[]): Counts {
  const counted = problems.filter((p) => p.level !== "info" && (p.origin === "new" || p.origin === "pending"));
  const origin = (o: Problem["origin"]) => problems.filter((p) => p.origin === o).length;
  return {
    headline: counted.length,
    worst: counted.some((p) => p.level === "error") ? "error" : counted.length ? "warning" : undefined,
    all: problems.length,
    newOnes: origin("new"),
    existing: origin("existing"),
    unknown: origin("unknown"),
    info: problems.filter((p) => p.level === "info").length,
  };
}
