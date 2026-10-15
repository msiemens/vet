// What Search Everywhere lists: matching and ranking of the five kinds of result, and how the
// ranked results are grouped for each tab. Pure: the candidates are made from the window's state by
// `paletteSources.ts`, and `Palette.svelte` only draws what `build` returns.

import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { SymbolKind } from "../../generated/SymbolKind";
import type { StatusLetter } from "../../lib/changeLetter.ts";
import { compareText } from "../../lib/format.ts";
import { basename } from "../../lib/paths.ts";
import { matchLowered, type Match, type Range } from "../../lib/speedSearch.ts";

export type Kind = "file" | "symbol" | "changeset" | "thread" | "action";
export type PaletteTab = "all" | Kind;

export const TABS: { id: PaletteTab; label: string }[] = [
  { id: "all", label: "All" },
  { id: "changeset", label: "Changesets" },
  { id: "file", label: "Files" },
  { id: "symbol", label: "Symbols" },
  { id: "thread", label: "Threads" },
  { id: "action", label: "Actions" },
];

/** The kinds in the order that settles a tie between groups of equal score. */
export const KINDS: readonly Kind[] = ["file", "symbol", "changeset", "thread", "action"];

export const TITLES: Record<Kind, string> = {
  file: "Files",
  symbol: "Symbols",
  changeset: "Changesets",
  thread: "Threads",
  action: "Actions",
};

/** How many results of a kind All shows before "Show all". */
export const CAPS: Record<Kind, number> = { file: 3, symbol: 3, changeset: 2, thread: 2, action: 3 };

/** How many recent results are remembered, and how many an empty All lists. */
export const RECENT_KEPT = 30;
const RECENT_SHOWN = 6;
/** How many of the least reviewed files an empty All lists. */
const LEAST_REVIEWED = 4;
/** How many results a group of a kind's own tab draws. */
const LIST_CAP = 200;

export interface FileCandidate {
  kind: "file";
  /** The path, which names the file. */
  key: string;
  path: string;
  /** The changeset changes it. */
  changed: boolean;
  status?: StatusLetter;
  added: number;
  removed: number;
  /** How much of the changes is seen, and how much only against an old base, in percent. */
  seen: number;
  oldBase: number;
}

export interface SymbolCandidate {
  kind: "symbol";
  key: string;
  info: SymbolInfo;
  symbolKind: SymbolKind;
  /** The name as the row shows it, and how much of its start is the container. */
  label: string;
  lead: number;
  /** Absent: a symbol the change left alone. */
  change?: SymbolChange;
  /** How much of its file is seen, in percent. */
  seen: number;
  /** A language server found it by name, so it is listed even where Vet's own match rejects it. */
  fromServer?: boolean;
}

export interface ChangesetCandidate {
  kind: "changeset";
  key: string;
  /** The number (`#12`) and the title, as the row names it. */
  number?: string;
  title: string;
  /** Where it lives. */
  source: string;
  icon: "branch" | "github";
  /** The one on view: choosing it does nothing. */
  onView: boolean;
  /** Progress of the review in percent; absent when none was started. */
  seen?: number;
  /** How much of it is seen only against an old base, in percent. */
  oldBase?: number;
  finished: boolean;
}

export interface ThreadCandidate {
  kind: "thread";
  key: string;
  /** The first message. */
  text: string;
  /** `path:line`, or `changeset`. */
  where: string;
  resolved: boolean;
  /** Its anchor moved or is ambiguous, and awaits confirmation. */
  moved: boolean;
}

export interface ActionCandidate {
  kind: "action";
  key: string;
  label: string;
  /** The key that runs it, as the viewer reads it. */
  binding?: string;
  /** Why it cannot run now, if it cannot. */
  disabled?: string;
  /** It cannot run now, whether or not the registry says why. */
  off: boolean;
}

export type Candidate = FileCandidate | SymbolCandidate | ChangesetCandidate | ThreadCandidate | ActionCandidate;

export type Candidates = { [K in Kind]: Extract<Candidate, { kind: K }>[] };

/** A result: the candidate, how well it matches, and where to highlight. */
export interface Row {
  item: Candidate;
  score: number;
  ranges: Range[];
  /** A `:line` typed after a file's name, 1-based. */
  line?: number;
}

/** What a list shows: a result, the way to a kind's whole list, or the way on to Find in Files. */
export type Entry =
  { type: "row"; row: Row } | { type: "more"; kind: Kind; count: number } | { type: "find"; query: string };

export interface Group {
  label: string;
  /** How many results the group has in all, shown beside its label in a tab of its own. */
  count?: number;
  /** What is being waited for: a spinner and these words stand in the group. */
  loading?: string;
  entries: Entry[];
  /** How many more results the group has than it lists. */
  cut?: number;
}

export interface Built {
  groups: Group[];
  /** What each tab has for the query; none without one. */
  counts?: Record<PaletteTab, number>;
}

/** Changed results lead unchanged ones by a margin larger than any word-start bonus of a match. */
const CHANGED = 40;
const UNREVIEWED = 6;
const OPEN = 6;
const NAME_OVER_PATH = 20;
const RECENT_BONUS = 6;
const RECENT_FALLOFF = 0.9;
const ON_VIEW = -50;
const DISABLED = -100;
/** Below every score a match can reach. */
const UNMATCHED = -1000;

/** The id a result is remembered by. */
export const recentId = (kind: Kind, key: string): string => `${kind}:${key}`;

/** The list after choosing a result: it comes first, and the list keeps `RECENT_KEPT`. */
export function remember(recent: readonly string[], id: string): string[] {
  return [id, ...recent.filter((r) => r !== id)].slice(0, RECENT_KEPT);
}

/** A file query with the line (and column) typed after its name: `retry.rs:40`. */
export function splitLine(query: string): { query: string; line?: number } {
  const found = /^(.*?):(\d+)(?::\d+)?$/.exec(query);
  return found ? { query: found[1], line: Number(found[2]) } : { query };
}

const unreviewed = (seen: number) => (seen < 100 ? UNREVIEWED : 0);

/** A candidate's text, lowercased once, and what else matching it needs. */
interface Prepared {
  /** The id of the row in the recent list. */
  id: string;
  /** The text its kind matches by: a file's name, else the name itself. */
  text: string;
  lower: string;
  /** A file's whole path, matched when its name does not; the name sits at its end, so ranges shift by `cut`. */
  path?: { text: string; lower: string };
  cut: number;
}

function prepare(item: Candidate): Prepared {
  const id = recentId(item.kind, item.key);
  switch (item.kind) {
    case "file": {
      const name = basename(item.path);
      return {
        id,
        text: name,
        lower: name.toLowerCase(),
        path: { text: item.path, lower: item.path.toLowerCase() },
        cut: item.path.length - name.length,
      };
    }
    case "symbol":
      return { id, text: item.label, lower: item.label.toLowerCase(), cut: 0 };
    case "changeset": {
      const text = item.number ? `${item.number} ${item.title}` : item.title;
      return { id, text, lower: text.toLowerCase(), cut: 0 };
    }
    case "thread":
      return { id, text: item.text, lower: item.text.toLowerCase(), cut: 0 };
    case "action":
      return { id, text: item.label, lower: item.label.toLowerCase(), cut: 0 };
  }
}

const prepared = new WeakMap<readonly Candidate[], Prepared[]>();

/** The candidates' texts, lowercased once and reused while the list itself is unchanged. */
function prepareAll(items: readonly Candidate[]): Prepared[] {
  const had = prepared.get(items);
  if (had?.length === items.length) return had;
  const list = items.map(prepare);
  prepared.set(items, list);
  return list;
}

/**
 * `matchLowered`, after a cheap look at whether the query's characters are in the text in order,
 * which every match needs: most of a project's files are rejected there, so tens of thousands cost
 * little. The two lowercased forms are the ones the list already keeps.
 */
function fuzzy(query: string, queryLower: string, text: string, textLower: string): Match | undefined {
  let at = 0;
  for (const c of queryLower) {
    at = textLower.indexOf(c, at) + 1;
    if (at === 0) return undefined;
  }
  return matchLowered(query, queryLower, text, textLower);
}

/** How the prepared `prep` matches `query`; `undefined` when it does not. */
function matchOf(prep: Prepared, query: string, queryLower: string): Match | undefined {
  const byName = fuzzy(query, queryLower, prep.text, prep.lower);
  if (prep.path === undefined) return byName;
  if (byName)
    return {
      score: byName.score + NAME_OVER_PATH,
      ranges: byName.ranges.map(([a, b]) => [a + prep.cut, b + prep.cut]),
    };
  return fuzzy(query, queryLower, prep.path.text, prep.path.lower);
}

/** What a result adds to its match for what it is. */
function weight(item: Candidate): number {
  switch (item.kind) {
    case "file":
      return item.changed ? CHANGED + unreviewed(item.seen) : 0;
    case "symbol":
      return item.change ? CHANGED + unreviewed(item.seen) : 0;
    case "changeset":
      return item.onView ? ON_VIEW : 0;
    case "thread":
      return item.resolved ? 0 : OPEN;
    case "action":
      return item.off ? DISABLED : 0;
  }
}

/** How a result of `item`'s kind is read when nothing is typed: the changed files least seen first. */
function emptyWeight(item: Candidate): number {
  return item.kind === "file" && item.changed ? 100 - item.seen : weight(item);
}

const labelOf = (item: Candidate): string => (item.kind === "action" ? item.label : item.key);

/** The best rows of a kind, and how many matched in all when only the best few are kept. */
export interface Ranked {
  rows: Row[];
  count: number;
}

/** Whether the row at `ai` sorts before the one at `bi`: score first, then the label (when the query or an action asks), then the order they came in. */
function before(a: Row, ai: number, b: Row, bi: number, byLabel: boolean): boolean {
  if (a.score !== b.score) return a.score > b.score;
  if (byLabel) {
    const order = compareText(labelOf(a.item), labelOf(b.item));
    if (order !== 0) return order < 0;
  }
  return ai < bi;
}

/** Keeps `rows`/`at` a heap whose root is the worst row kept, so a better one can replace it. */
function siftDown(rows: Row[], at: number[], byLabel: boolean, i: number): void {
  for (;;) {
    let worst = i;
    const left = 2 * i + 1;
    const right = left + 1;
    if (left < rows.length && before(rows[worst], at[worst], rows[left], at[left], byLabel)) worst = left;
    if (right < rows.length && before(rows[worst], at[worst], rows[right], at[right], byLabel)) worst = right;
    if (worst === i) return;
    [rows[i], rows[worst]] = [rows[worst], rows[i]];
    [at[i], at[worst]] = [at[worst], at[i]];
    i = worst;
  }
}

function heapify(rows: Row[], at: number[], byLabel: boolean): void {
  for (let i = (rows.length >> 1) - 1; i >= 0; i--) siftDown(rows, at, byLabel, i);
}

/**
 * The candidates of one kind that match `query`, best first, and how many matched. A query matches
 * the way speed search does; what the match is worth is raised by what the review needs first
 * (changed, not yet seen, open) and a little by what was chosen recently, which only decides between
 * close matches. Nothing typed lists everything, in the order the sources gave except for files,
 * which come least seen first. A symbol a language server found stays listed without a match, below
 * all that match. With `keep` only that many of the best rows are kept (a bounded heap), and `only`
 * narrows the list to one part of it, such as the changed files.
 */
export function rank(
  items: readonly Candidate[],
  query: string,
  recent: readonly string[],
  keep = Infinity,
  only?: (item: Candidate) => boolean,
): Ranked {
  const typed = items[0]?.kind === "file" ? splitLine(query.trim()) : { query: query.trim(), line: undefined };
  const q = typed.query;
  const qLower = q.toLowerCase();
  const preps = prepareAll(items);
  const byLabel = q !== "" || items[0]?.kind === "action";
  // Where each recently chosen id sits, the first of equals, as `indexOf` would find it.
  const recentAt = new Map<string, number>();
  for (let i = 0; i < recent.length; i++) if (!recentAt.has(recent[i])) recentAt.set(recent[i], i);

  const bounded = keep < items.length;
  const rows: Row[] = [];
  const at: number[] = [];
  let count = 0;
  for (let index = 0; index < items.length; index++) {
    const item = items[index];
    if (only && !only(item)) continue;
    const matched = q ? matchOf(preps[index], q, qLower) : { score: 0, ranges: [] };
    const unmatched = !matched && item.kind === "symbol" && item.fromServer;
    const found = matched ?? (unmatched ? { score: UNMATCHED, ranges: [] } : undefined);
    if (!found) continue;
    count++;
    const base = q ? weight(item) : emptyWeight(item);
    const recentIndex = recentAt.get(preps[index].id);
    const bonus = recentIndex === undefined ? 0 : RECENT_BONUS * RECENT_FALLOFF ** recentIndex;
    const row: Row = { item, score: found.score + base + bonus, ranges: found.ranges, line: typed.line };
    if (!bounded) {
      rows.push(row);
    } else if (rows.length < keep) {
      rows.push(row);
      at.push(index);
      if (rows.length === keep) heapify(rows, at, byLabel);
    } else if (before(row, index, rows[0], at[0], byLabel)) {
      rows[0] = row;
      at[0] = index;
      siftDown(rows, at, byLabel, 0);
    }
  }

  if (!bounded) {
    const byScore = (a: Row, b: Row) => b.score - a.score;
    if (byLabel) rows.sort((a, b) => byScore(a, b) || compareText(labelOf(a.item), labelOf(b.item)));
    else rows.sort(byScore);
    return { rows, count };
  }
  const kept = rows.map((row, i) => ({ row, at: at[i] }));
  kept.sort((a, b) => (before(a.row, a.at, b.row, b.at, byLabel) ? -1 : 1));
  return { rows: kept.map((k) => k.row), count };
}

/** The results the recent list names that the sources still have, newest first. */
function recentRows(recent: readonly string[], candidates: Candidates, kinds: readonly Kind[]): Row[] {
  const rows: Row[] = [];
  for (const id of recent) {
    if (rows.length === RECENT_SHOWN) break;
    const kind = KINDS.find((k) => id.startsWith(`${k}:`));
    if (!kind || !kinds.includes(kind)) continue;
    const item = (candidates[kind] as Candidate[]).find((c) => recentId(kind, c.key) === id);
    if (item) rows.push({ item, score: 0, ranges: [] });
  }
  return rows;
}

const rowsOf = (rows: readonly Row[]): Entry[] => rows.map((row) => ({ type: "row", row }));

/** A group of a kind's own tab that already keeps only the best few. */
const listed = (label: string, ranked: Ranked): Group => ({
  label,
  count: ranked.count,
  entries: rowsOf(ranked.rows),
  cut: Math.max(0, ranked.count - ranked.rows.length),
});

/** A group of a kind's own tab, from rows that are all there is. */
const listedRows = (label: string, rows: readonly Row[]): Group => ({
  label,
  count: rows.length,
  entries: rowsOf(rows.slice(0, LIST_CAP)),
  cut: Math.max(0, rows.length - LIST_CAP),
});

/** Whether the file row `a` sorts before `b`, the order `rank` settles ties in. */
const fileBefore = (a: Row, b: Row): boolean =>
  a.score !== b.score ? a.score > b.score : compareText(labelOf(a.item), labelOf(b.item)) < 0;

/** The changed and unchanged files merged, best first. Their paths are unique, so the two sorted lists interleave. */
function mergeFiles(changed: Ranked, other: Ranked): Ranked {
  const rows: Row[] = [];
  let i = 0;
  let j = 0;
  while (rows.length < LIST_CAP && (i < changed.rows.length || j < other.rows.length)) {
    const takeChanged =
      j >= other.rows.length || (i < changed.rows.length && fileBefore(changed.rows[i], other.rows[j]));
    rows.push(takeChanged ? changed.rows[i++] : other.rows[j++]);
  }
  return { rows, count: changed.count + other.count };
}

/** The place of the files that are still being listed. */
const LISTING: Group = { label: "Other files", loading: "Listing the project’s files…", entries: [] };

const ASKING = "Asking the language servers…";

/** What is still being waited for. */
export interface Waiting {
  /** The list of all files. */
  files?: boolean;
  /** The language servers' answer to the query. */
  symbols?: boolean;
}

/**
 * What a tab lists for `query`. All shows each kind's best few, the kinds ordered by their best
 * result, each ending in the way to its tab; nothing typed shows the recent results and the changed
 * files least seen. A kind's own tab lists all its results, grouped where that says something. With a
 * query the lists end in the way on to Find in Files. A group still waiting for an answer says so.
 * A query ranks only the rows a tab can draw (the best few), the files' changed and unchanged apart.
 */
export function build(
  tab: PaletteTab,
  query: string,
  candidates: Candidates,
  recent: readonly string[],
  waiting: Waiting = {},
): Built {
  const q = query.trim();
  // A file query may be only a line, `:12`, which matches everything: then the order is the sources', not a label's.
  const byFileQuery = splitLine(q).query !== "";
  const fileChanged = byFileQuery
    ? rank(candidates.file, q, recent, LIST_CAP, (c) => c.kind === "file" && c.changed)
    : undefined;
  const fileOther = byFileQuery
    ? rank(candidates.file, q, recent, LIST_CAP, (c) => c.kind === "file" && !c.changed)
    : undefined;
  const ranked: Record<Kind, Ranked> = {
    file: fileChanged && fileOther ? mergeFiles(fileChanged, fileOther) : rank(candidates.file, q, recent),
    symbol: rank(candidates.symbol, q, recent, LIST_CAP),
    changeset: rank(candidates.changeset, q, recent),
    thread: rank(candidates.thread, q, recent),
    action: rank(candidates.action, q, recent),
  };
  const groups: Group[] = [];
  const find: Group = { label: "", entries: [{ type: "find", query: q }] };

  if (tab === "all") {
    if (!q) {
      const latest = recentRows(recent, candidates, KINDS);
      if (latest.length) groups.push({ label: "Recent", entries: rowsOf(latest) });
      const least = ranked.file.rows.filter((r) => r.item.kind === "file" && r.item.changed && r.item.seen < 100);
      if (least.length)
        groups.push({ label: "Changed files, least reviewed first", entries: rowsOf(least.slice(0, LEAST_REVIEWED)) });
      return { groups };
    }
    const kinds = KINDS.flatMap((kind, order) => {
      const best = ranked[kind];
      if (!best.rows.length) return [];
      const entries = rowsOf(best.rows.slice(0, CAPS[kind]));
      if (best.count > CAPS[kind]) entries.push({ type: "more", kind, count: best.count });
      const group: Group = {
        label: TITLES[kind],
        entries,
        loading: kind === "symbol" && waiting.symbols ? ASKING : undefined,
      };
      return [{ order, best: best.rows[0].score, group }];
    });
    kinds.sort((a, b) => b.best - a.best || a.order - b.order);
    groups.push(...kinds.map((k) => k.group));
    if (waiting.symbols && !ranked.symbol.count) groups.push({ label: TITLES.symbol, loading: ASKING, entries: [] });
    if (waiting.files) groups.push(LISTING);
    if (groups.length) groups.push(find);
    return { groups, counts: counts(ranked) };
  }

  if (tab === "file") {
    if (fileChanged && fileOther) {
      groups.push(listed("Changed files", fileChanged), listed("Other files", fileOther));
    } else {
      const latest = q ? [] : recentRows(recent, candidates, ["file"]);
      const taken = new Set(latest.map((r) => r.item.key));
      const fresh = ranked.file.rows.filter((r) => !taken.has(r.item.key));
      if (latest.length) groups.push({ label: "Recent", entries: rowsOf(latest) });
      groups.push(
        listedRows(
          "Changed files",
          fresh.filter((r) => r.item.kind === "file" && r.item.changed),
        ),
        listedRows(
          "Other files",
          fresh.filter((r) => r.item.kind === "file" && !r.item.changed),
        ),
      );
    }
  } else if (!q && tab === "action") {
    const latest = recentRows(recent, candidates, ["action"]);
    const taken = new Set(latest.map((r) => r.item.key));
    if (latest.length) groups.push({ label: "Recent", entries: rowsOf(latest) });
    groups.push({
      label: "All actions",
      count: ranked.action.count,
      entries: rowsOf(ranked.action.rows.filter((r) => !taken.has(r.item.key))),
    });
  } else if (!q && tab === "thread") {
    const open = ranked.thread.rows.filter((r) => r.item.kind === "thread" && !r.item.resolved);
    const resolved = ranked.thread.rows.filter((r) => r.item.kind === "thread" && r.item.resolved);
    groups.push(
      { label: "Open", count: open.length, entries: rowsOf(open) },
      { label: "Resolved", count: resolved.length, entries: rowsOf(resolved) },
    );
  } else if (tab === "symbol") {
    groups.push({
      label: q ? "Symbols" : "Changed symbols",
      count: ranked.symbol.count,
      entries: rowsOf(ranked.symbol.rows),
      cut: Math.max(0, ranked.symbol.count - ranked.symbol.rows.length),
      loading: waiting.symbols ? ASKING : undefined,
    });
  } else {
    groups.push({ label: TITLES[tab], count: ranked[tab].count, entries: rowsOf(ranked[tab].rows) });
  }
  const shown = groups.filter((g) => g.entries.length || g.loading);
  if (waiting.files && tab === "file") shown.push(LISTING);
  if (q && tab !== "action" && shown.length) shown.push(find);
  return { groups: shown, counts: q ? counts(ranked) : undefined };
}

function counts(ranked: Record<Kind, Ranked>): Record<PaletteTab, number> {
  const per = Object.fromEntries(KINDS.map((k) => [k, ranked[k].count])) as Record<Kind, number>;
  return { ...per, all: KINDS.reduce((sum, k) => sum + per[k], 0) };
}

/** The entries a cursor can rest on, in display order. */
export const entriesOf = (groups: readonly Group[]): Entry[] => groups.flatMap((g) => g.entries);
