// What Find in Files shows, purely: the places it can search, the results as rows of a list, and the
// preview of a result as the unified diff draws it.

import type { Hunk } from "../../generated/Hunk.ts";
import type { SearchFile } from "../../generated/SearchFile.ts";
import type { SearchHit } from "../../generated/SearchHit.ts";
import type { SearchLine } from "../../generated/SearchLine.ts";
import type { SearchLineKind } from "../../generated/SearchLineKind.ts";
import type { SearchScope } from "../../generated/SearchScope.ts";
import type { View } from "../../generated/View.ts";
import { type Highlights, pieces } from "../../lib/highlights.ts";

/** A place to search, as the scope menu lists it. */
export interface ScopeChoice {
  /** Stable while the place exists: what is kept between openings. */
  id: string;
  label: string;
  detail: string;
  group: "view" | "commit" | "repository";
  scope: SearchScope;
}

export interface ScopeSources {
  /** The view the review shows. */
  view: View;
  /** How many files the changeset changes. */
  files: number;
  /** The versions to compare, if the review has more than one. */
  range?: { from: number; to: number };
  /** The commits of the changeset, oldest first. */
  commits: { shortId: string; subject: string }[];
}

export const CHANGES_SCOPE = "changes";

const rangeId = (from: number, to: number) => `versions:${from}:${to}`;
const commitId = (index: number) => `commit:${index}`;

/** The id of the choice that stands for `view`. */
export function viewScopeId(view: View): string {
  if (view.type === "interdiff") return rangeId(view.from, view.to);
  return view.type === "commit" ? commitId(view.index) : CHANGES_SCOPE;
}

export function scopeChoices(s: ScopeSources): ScopeChoice[] {
  const onShow = viewScopeId(s.view);
  const note = (id: string, detail: string) => (id === onShow && id !== CHANGES_SCOPE ? "on show" : detail);
  const choices: ScopeChoice[] = [
    {
      id: CHANGES_SCOPE,
      label: "All changes",
      detail: `${s.files} changed ${s.files === 1 ? "file" : "files"}`,
      group: "view",
      scope: { type: "view", view: { type: "changeset" } },
    },
  ];
  if (s.range) {
    const { from, to } = s.range;
    const id = rangeId(from, to);
    choices.push({
      id,
      label: `Versions v${from} → v${to}`,
      detail: note(id, "what changed between them"),
      group: "view",
      scope: { type: "view", view: { type: "interdiff", from, to } },
    });
  }
  s.commits.forEach((c, index) => {
    const id = commitId(index);
    choices.push({
      id,
      label: `Commit ${c.shortId}`,
      detail: note(id, c.subject),
      group: "commit",
      scope: { type: "view", view: { type: "commit", index } },
    });
  });
  choices.push({
    id: "repository",
    label: "Whole repository",
    detail: "every file of the target, ignored ones apart",
    group: "repository",
    scope: { type: "repository" },
  });
  return choices;
}

/** The choice with this id; All changes when the place is gone (its commit or versions left the changeset). */
export function resolveScope(choices: readonly ScopeChoice[], id: string): ScopeChoice {
  return choices.find((c) => c.id === id) ?? choices[0];
}

/** The query as the dialog searched it, which the find bar carries on in the view the result opens in. */
export interface OpenedQuery {
  text: string;
  matchCase: boolean;
  wholeWord: boolean;
  regex: boolean;
  side: "both" | "target" | "base";
  onlyChanges: boolean;
}

/** A result the viewer opened. */
export interface OpenedResult {
  path: string;
  /** The hunks of the file, as the search saw them. */
  hunks: readonly Hunk[];
  kind: SearchLineKind;
  /** 0-based: of the target for a context or added line, of the base for a removed one. */
  line: number;
  /** The first match's column in the line and its width, in UTF-16 units. */
  col: number;
  width: number;
  /** The full id of the commit searched, for a search of one commit. */
  commit?: string;
  /** Show it in All changes: the place searched is the view on show. */
  reveal: boolean;
  options: OpenedQuery;
}

/** A row of the results list: a file's header, or one of its matching lines. */
export type ResultRow =
  | { kind: "file"; file: SearchFile; fileIndex: number; collapsed: boolean }
  | { kind: "line"; file: SearchFile; fileIndex: number; line: SearchLine; lineIndex: number };

export const rowKey = (row: ResultRow): string =>
  row.kind === "file" ? `f:${row.file.path}` : `l:${row.file.path}:${row.lineIndex}`;

/** The id of a result across searches: its file and its line's place in the file's results. */
export const resultId = (path: string, lineIndex: number): string => `${path}\0${lineIndex}`;

export function resultRows(files: readonly SearchFile[], collapsed: ReadonlySet<string>): ResultRow[] {
  return files.flatMap((file, fileIndex) => {
    const shut = collapsed.has(file.path);
    const header: ResultRow = { kind: "file", file, fileIndex, collapsed: shut };
    if (shut) return [header];
    return [
      header,
      ...file.lines.map((line, lineIndex): ResultRow => ({ kind: "line", file, fileIndex, line, lineIndex })),
    ];
  });
}

/** The index of the line row `delta` lines from the one at `from` (none: the first or last); headers are skipped. */
export function stepLine(rows: readonly ResultRow[], from: number, delta: 1 | -1): number | undefined {
  for (let i = from < 0 ? (delta > 0 ? 0 : rows.length - 1) : from + delta; i >= 0 && i < rows.length; i += delta) {
    if (rows[i].kind === "line") return i;
  }
  return undefined;
}

/** The row of the result `id`, else the first line row. */
export function selectedRow(rows: readonly ResultRow[], id: string | undefined): number {
  const at = rows.findIndex((r) => r.kind === "line" && resultId(r.file.path, r.lineIndex) === id);
  return at >= 0 ? at : (stepLine(rows, -1, 1) ?? -1);
}

/** What the line starts with is left out of its row; the matches' columns move with it. */
export function trimmed(line: SearchLine): { text: string; hits: SearchHit[] } {
  const lead = line.text.length - line.text.trimStart().length;
  return {
    text: line.text.slice(lead),
    hits: line.hits
      .map((h) => ({ from: h.from - line.textStart - lead, to: h.to - line.textStart - lead }))
      .filter((h) => h.to > 0)
      .map((h) => ({ from: Math.max(0, h.from), to: h.to })),
  };
}

const LINE_BREAK = /\r\n?|\n/;

/** A line of the preview, as the unified diff draws it. */
export interface PreviewRow {
  kind: SearchLineKind;
  /** 1-based numbers on each side; a removed line has no target number, an added one no base number. */
  oldNo?: number;
  newNo?: number;
  text: string;
  /** Where the line starts in the text of the side it belongs to, for the syntax colours. */
  start: number;
  /** Which text the line is of: a removed line is the base's. */
  side: "base" | "target";
  /** The hunk of an added or removed line. */
  hunk?: number;
  at: boolean;
}

export interface PreviewInput {
  base: string;
  target: string;
  hunks: readonly Hunk[];
  /** The result: its kind and 0-based line. */
  kind: SearchLineKind;
  line: number;
  /** How many lines of context to show on each side. */
  span: number;
  /** Without the base side, the target is shown alone: a removed line is not drawn. */
  withBase: boolean;
}

function split(text: string): { lines: string[]; starts: number[] } {
  const lines = text.split(LINE_BREAK);
  const starts: number[] = [];
  let at = 0;
  for (const l of lines) {
    starts.push(at);
    at += l.length + 1;
  }
  return { lines, starts };
}

/** The lines of the unified diff around the result: the context lines, then a hunk's removed lines, then its added ones. */
export function previewRows(input: PreviewInput): PreviewRow[] {
  const t = split(input.target);
  const b = input.withBase ? split(input.base) : { lines: [] as string[], starts: [] as number[] };
  const rows: PreviewRow[] = [];
  let hit = -1;
  const same = (kind: SearchLineKind, line: number) => kind === input.kind && line === input.line;
  const push = (row: Omit<PreviewRow, "at">) => {
    const at = same(row.kind, row.kind === "removed" ? (row.oldNo ?? 0) - 1 : (row.newNo ?? 0) - 1);
    if (at) hit = rows.length;
    rows.push({ ...row, at });
  };
  let nt = 0;
  let nb = 0;
  const context = (until: number) => {
    for (; nt < until && nt < t.lines.length; nt++, nb++)
      push({ kind: "context", oldNo: nb + 1, newNo: nt + 1, text: t.lines[nt], start: t.starts[nt], side: "target" });
  };
  input.hunks.forEach((h, i) => {
    context(h.targetStart);
    nb = h.baseStart;
    if (input.withBase)
      for (let j = 0; j < h.baseLen && nb + j < b.lines.length; j++) {
        const at = nb + j;
        push({ kind: "removed", oldNo: at + 1, text: b.lines[at], start: b.starts[at], side: "base", hunk: i });
      }
    for (let j = 0; j < h.targetLen && h.targetStart + j < t.lines.length; j++) {
      const at = h.targetStart + j;
      push({ kind: "added", newNo: at + 1, text: t.lines[at], start: t.starts[at], side: "target", hunk: i });
    }
    nb = h.baseStart + h.baseLen;
    nt = h.targetStart + h.targetLen;
  });
  context(t.lines.length);
  if (hit < 0) return [];
  return rows.slice(Math.max(0, hit - input.span), hit + input.span + 1);
}

/** A stretch of a line with its syntax colour and whether a match covers it. */
export interface Segment {
  text: string;
  className?: string;
  find?: "hit" | "current";
}

/**
 * `text`, which starts at `start` in the text `highlights` colour, cut where the colour or a match
 * changes. `hits` are columns of `text`; the one at `current` is the result being looked at.
 */
export function segments(
  text: string,
  start: number,
  highlights: Highlights,
  hits: readonly SearchHit[],
  current?: number,
): Segment[] {
  const out: Segment[] = [];
  let at = 0;
  for (const piece of pieces(highlights, start, text)) {
    const [chunk, className] = typeof piece === "string" ? [piece, undefined] : [piece.text, piece.className];
    const end = at + chunk.length;
    let cut = at;
    // The matches that overlap this piece, in order, cut it up.
    for (const [i, h] of hits.entries()) {
      if (h.to <= cut || h.from >= end) continue;
      const from = Math.max(h.from, cut);
      const to = Math.min(h.to, end);
      if (from > cut) out.push({ text: text.slice(cut, from), className });
      out.push({ text: text.slice(from, to), className, find: i === current ? "current" : "hit" });
      cut = to;
    }
    if (cut < end) out.push({ text: text.slice(cut, end), className });
    at = end;
  }
  return out;
}
