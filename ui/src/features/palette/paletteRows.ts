// What Search Everywhere lists: matching and ranking of the five kinds of result, and how the
// ranked results are grouped for each tab. Pure: the candidates are made from the window's state by
// `paletteSources.ts`, and `Palette.svelte` only draws what `build` returns.

import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { SymbolKind } from "../../generated/SymbolKind";
import { basename } from "../../lib/paths.ts";
import { match, type Range } from "../../lib/speedSearch.ts";

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

export interface FileCandidate {
  kind: "file";
  /** The path, which names the file. */
  key: string;
  path: string;
  /** The changeset changes it. */
  changed: boolean;
  status?: "M" | "A" | "D" | "R";
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
  entries: Entry[];
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

const collator = new Intl.Collator();

/** The id a result is remembered by. */
export const recentId = (kind: Kind, key: string): string => `${kind}:${key}`;

/** The list after choosing a result: it comes first, and the list keeps `RECENT_KEPT`. */
export function remember(recent: readonly string[], id: string): string[] {
  return [id, ...recent.filter((r) => r !== id)].slice(0, RECENT_KEPT);
}

/** What a recently chosen result adds to a match: more the more recently it was chosen. */
function recentBonus(recent: readonly string[], item: Candidate): number {
  const at = recent.indexOf(recentId(item.kind, item.key));
  return at < 0 ? 0 : RECENT_BONUS * RECENT_FALLOFF ** at;
}

/** A file query with the line (and column) typed after its name: `retry.rs:40`. */
export function splitLine(query: string): { query: string; line?: number } {
  const found = /^(.*?):(\d+)(?::\d+)?$/.exec(query);
  return found ? { query: found[1], line: Number(found[2]) } : { query };
}

const unreviewed = (seen: number) => (seen < 100 ? UNREVIEWED : 0);

/** How `item` matches `query`; `undefined` when it does not. */
function matchOf(item: Candidate, query: string): { score: number; ranges: Range[] } | undefined {
  switch (item.kind) {
    case "file": {
      const name = basename(item.path);
      const cut = item.path.length - name.length;
      const byName = match(query, name);
      if (byName)
        return { score: byName.score + NAME_OVER_PATH, ranges: byName.ranges.map(([a, b]) => [a + cut, b + cut]) };
      return match(query, item.path);
    }
    case "symbol":
      return match(query, item.label);
    case "changeset":
      return match(query, item.number ? `${item.number} ${item.title}` : item.title);
    case "thread":
      return match(query, item.text);
    case "action":
      return match(query, item.label);
  }
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

/**
 * The candidates of one kind that match `query`, best first. A query matches the way speed search
 * does; what the match is worth is raised by what the review needs first (changed, not yet seen,
 * open) and a little by what was chosen recently, which only decides between close matches. Nothing
 * typed lists everything, in the order the sources gave except for files, which come least seen first.
 */
export function rank(items: readonly Candidate[], query: string, recent: readonly string[]): Row[] {
  const rows: Row[] = [];
  const typed = items[0]?.kind === "file" ? splitLine(query.trim()) : { query: query.trim(), line: undefined };
  for (const item of items) {
    const found = typed.query ? matchOf(item, typed.query) : { score: 0, ranges: [] };
    if (!found) continue;
    const base = typed.query ? weight(item) : emptyWeight(item);
    rows.push({ item, score: found.score + base + recentBonus(recent, item), ranges: found.ranges, line: typed.line });
  }
  const byScore = (a: Row, b: Row) => b.score - a.score;
  if (typed.query || items[0]?.kind === "action") {
    rows.sort((a, b) => byScore(a, b) || collator.compare(labelOf(a.item), labelOf(b.item)));
  } else {
    rows.sort(byScore);
  }
  return rows;
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

/**
 * What a tab lists for `query`. All shows each kind's best few, the kinds ordered by their best
 * result, each ending in the way to its tab; nothing typed shows the recent results and the changed
 * files least seen. A kind's own tab lists all its results, grouped where that says something. With a
 * query the lists end in the way on to Find in Files.
 */
export function build(tab: PaletteTab, query: string, candidates: Candidates, recent: readonly string[]): Built {
  const q = query.trim();
  const ranked = Object.fromEntries(KINDS.map((k) => [k, rank(candidates[k], q, recent)])) as Record<Kind, Row[]>;
  const groups: Group[] = [];
  const find: Group = { label: "", entries: [{ type: "find", query: q }] };

  if (tab === "all") {
    if (!q) {
      const latest = recentRows(recent, candidates, KINDS);
      if (latest.length) groups.push({ label: "Recent", entries: rowsOf(latest) });
      const least = ranked.file.filter((r) => r.item.kind === "file" && r.item.changed && r.item.seen < 100);
      if (least.length)
        groups.push({ label: "Changed files, least reviewed first", entries: rowsOf(least.slice(0, LEAST_REVIEWED)) });
      return { groups };
    }
    const kinds = KINDS.flatMap((kind, order) => {
      const rows = ranked[kind];
      if (!rows.length) return [];
      const entries = rowsOf(rows.slice(0, CAPS[kind]));
      if (rows.length > CAPS[kind]) entries.push({ type: "more", kind, count: rows.length });
      return [{ order, best: rows[0].score, group: { label: TITLES[kind], entries } }];
    });
    kinds.sort((a, b) => b.best - a.best || a.order - b.order);
    groups.push(...kinds.map((k) => k.group));
    if (groups.length) groups.push(find);
    return { groups, counts: counts(ranked) };
  }

  const rows = ranked[tab];
  if (!q && tab === "file") {
    const latest = recentRows(recent, candidates, ["file"]);
    const taken = new Set(latest.map((r) => r.item.key));
    const rest = rows.filter((r) => !taken.has(r.item.key));
    if (latest.length) groups.push({ label: "Recent", entries: rowsOf(latest) });
    groups.push({ label: "Changed files", count: rows.length, entries: rowsOf(rest) });
  } else if (!q && tab === "action") {
    const latest = recentRows(recent, candidates, ["action"]);
    const taken = new Set(latest.map((r) => r.item.key));
    if (latest.length) groups.push({ label: "Recent", entries: rowsOf(latest) });
    groups.push({
      label: "All actions",
      count: rows.length,
      entries: rowsOf(rows.filter((r) => !taken.has(r.item.key))),
    });
  } else if (!q && tab === "thread") {
    const open = rows.filter((r) => r.item.kind === "thread" && !r.item.resolved);
    const resolved = rows.filter((r) => r.item.kind === "thread" && r.item.resolved);
    groups.push(
      { label: "Open", count: open.length, entries: rowsOf(open) },
      { label: "Resolved", count: resolved.length, entries: rowsOf(resolved) },
    );
  } else if (tab === "symbol") {
    groups.push({ label: q ? "Symbols" : "Changed symbols", count: rows.length, entries: rowsOf(rows) });
  } else {
    groups.push({ label: TITLES[tab], count: rows.length, entries: rowsOf(rows) });
  }
  const shown = groups.filter((g) => g.entries.length);
  if (q && tab !== "action" && shown.length) shown.push(find);
  return { groups: shown, counts: q ? counts(ranked) : undefined };
}

function counts(ranked: Record<Kind, Row[]>): Record<PaletteTab, number> {
  const per = Object.fromEntries(KINDS.map((k) => [k, ranked[k].length])) as Record<Kind, number>;
  return { ...per, all: KINDS.reduce((sum, k) => sum + per[k], 0) };
}

/** The entries a cursor can rest on, in display order. */
export const entriesOf = (groups: readonly Group[]): Entry[] => groups.flatMap((g) => g.entries);
