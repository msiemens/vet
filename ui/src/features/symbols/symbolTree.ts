// Pure model of the Symbol panel's trees: what each root lists, as rows that nest. The component
// draws them and keeps which are open; here are the grouping, the order, the test detection and
// the words (SPEC §7.3).

import type { CallSite } from "../../generated/CallSite";
import type { Calls } from "../../generated/Calls";
import type { ChangedCaller } from "../../generated/ChangedCaller";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { Position } from "../../generated/Position";
import type { Reach } from "../../generated/Reach";
import type { Reference } from "../../generated/Reference";
import type { RemovalImpact } from "../../generated/RemovalImpact";
import type { Side } from "../../generated/Side";
import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import type { SymbolKind } from "../../generated/SymbolKind";
import type { SymbolName } from "../../generated/SymbolName";
import type { Usages } from "../../generated/Usages";
import { containersOf, fileName, partsOf, shortLabel } from "../../lib/symbolLabel.ts";
import { describeFate, type FateTone, rowTitle } from "./symbolDetail.ts";

/** Top-level rows a root shows before "N more". */
export const ROWS = 5;

/** What a click on a row does. */
export type Target =
  | { type: "open"; path: string; line?: number; side: Side }
  /** A changed symbol: put on show, revealed in the Changes tree and opened where it is. */
  | { type: "choose"; symbol: SymbolInfo; change: SymbolChange };

/** One row of a root, with the rows under it. */
export interface Entry {
  /** Names the row among the rows of its root. */
  key: string;
  /** The box before the name; a branch has none. */
  glyph?: SymbolKind | "file";
  name: string;
  /** The name is a file's, not code. */
  file?: boolean;
  /** The type the member is in, after its name. */
  container?: string;
  /** The file's name, which only a wide panel shows. */
  loc?: string;
  /** `:25 :31`, the lines of the places. */
  lines?: string;
  change?: SymbolChange;
  gone?: boolean;
  /** What became of a reference to a removed symbol. */
  fate?: { label: string; tone: FateTone };
  /** A second line. */
  sub?: string;
  /** Softer: a test, or something already handled. */
  muted?: boolean;
  /** A heading for the rows under it rather than a place. */
  branch?: boolean;
  /** Open before the reviewer says otherwise. */
  open?: boolean;
  /** Its rows always show; it cannot be folded. */
  fixed?: boolean;
  /** What is behind the row while it is shut. */
  count: number;
  title: string;
  /** None for a place outside the repository. */
  go?: Target;
  children: Entry[];
}

export interface Note {
  text: string;
  /** Says the list is incomplete. */
  warn?: boolean;
}

/** What a root has to show once its answer is in. */
export interface Body {
  /** In the root's title line. */
  count?: number;
  meta?: string;
  notes: Note[];
  /** The rows that stop after `ROWS`. */
  entries: Entry[];
  /** The rows after them, which are not cut. */
  tail: Entry[];
  /** Nothing to show: this one muted line stands in place of the root. */
  quiet?: string;
}

const TEST_PATH = [
  /(^|\/)(tests?|__tests__)\//i,
  /(^|\/)[^/]*\.tests?\//i,
  /(Tests?|Spec)\.(cs|java|kt|scala)$/,
  /\.(test|spec)\.[cm]?[jt]sx?$/,
  /_test\.(rs|go|py)$/,
  /(^|\/)test_[^/]*\.py$/,
];

/** A test by the conventions of the languages: where its file is and what it is called, or a `tests` module. */
export const isTest = (path: string, containers: readonly string[] = []): boolean =>
  TEST_PATH.some((p) => p.test(path)) || containers.some((c) => c === "tests" || c === "test");

const open = (path: string, line: number | undefined, side: Side): Target => ({ type: "open", path, line, side });

/** The file's name for a wide row, unless the container already says it (`Client` in `client.rs`). */
function locOf(path: string, container?: string): string | undefined {
  if (container === undefined) return undefined;
  const file = fileName(path);
  const stem = file.replace(/\.\w+$/, "");
  return stem.replaceAll("_", "").toLowerCase() === container.toLowerCase() ? undefined : file;
}

const lineText = (line: number): string => `:${line + 1}`;

function symbolEntry(key: string, symbol: SymbolInfo, change: SymbolChange | undefined, go: Target): Entry {
  const { name, container } = partsOf(symbol);
  const { path } = symbol.id;
  return {
    key,
    glyph: symbol.kind,
    name,
    container,
    loc: locOf(path, container),
    lines: lineText(symbol.at.line),
    change,
    gone: change === "removed",
    muted: isTest(path, containersOf(symbol.id)),
    count: 0,
    title: rowTitle(symbol.id, path, symbol.at),
    go,
    children: [],
  };
}

/** Production code before tests, each in the order given; a container is named once for a run of rows. */
function arrange(entries: Entry[]): Entry[] {
  const sorted = [...entries.filter((e) => !e.muted), ...entries.filter((e) => e.muted)];
  let previous: string | undefined;
  for (const e of sorted) {
    const same = e.container !== undefined && e.container === previous;
    previous = e.container;
    if (same) e.container = undefined;
  }
  return sorted;
}

/** The rows of `entries` and under them. */
export const countRows = (entries: readonly Entry[]): number =>
  entries.reduce((n, e) => n + 1 + countRows(e.children), 0);

/** The places behind `entries`: a row without rows under it is one. */
const countLeaves = (entries: readonly Entry[]): number =>
  entries.reduce((n, e) => n + (e.children.length ? countLeaves(e.children) : 1), 0);

/** The changed callers as a tree: a caller's callers are under it. The walk lists each before its callers. */
export function callerTree(callers: readonly ChangedCaller[]): Entry[] {
  const roots: Entry[] = [];
  const ancestors: Entry[] = [];
  callers.forEach((c, i) => {
    const entry = symbolEntry(`c${i}`, c.symbol, c.change, { type: "choose", symbol: c.symbol, change: c.change });
    const parent = c.depth > 0 ? ancestors[c.depth - 1] : undefined;
    (parent?.children ?? roots).push(entry);
    ancestors.length = c.depth;
    ancestors[c.depth] = entry;
  });
  const settle = (list: Entry[]): Entry[] => {
    for (const e of list) {
      e.children = settle(e.children);
      e.count = countRows(e.children);
    }
    return arrange(list);
  };
  return settle(roots);
}

export function callersBody(calls: Calls): Body {
  const entries = callerTree(calls.callers);
  const notes = calls.gaps.length ? [{ text: incomplete(calls.gaps), warn: true }] : [];
  return {
    count: countRows(entries),
    notes,
    entries,
    tail: [],
    quiet: entries.length || notes.length ? undefined : "No changed callers",
  };
}

/** What an empty list says when the analysis behind it has gaps: that it is incomplete, not that nothing is there. */
const incomplete = (gaps: readonly string[]): string => `Incomplete: ${gaps.join("; ")}`;

export function calleesBody(callees: readonly ChangedSymbol[], gaps: readonly string[]): Body {
  const entries = arrange(
    callees.map((c, i) =>
      symbolEntry(`e${i}`, c.symbol, c.change, { type: "choose", symbol: c.symbol, change: c.change }),
    ),
  );
  const empty = gaps.length ? incomplete(gaps) : "No changed callees";
  return {
    count: entries.length,
    notes: gaps.length && entries.length ? [{ text: incomplete(gaps), warn: true }] : [],
    entries,
    tail: [],
    quiet: entries.length ? undefined : empty,
  };
}

/** A place that refers to a symbol, or is one of its callers. */
interface Place {
  path: string;
  at: Position;
  within: SymbolName | null;
  /** What `within` is, where the answer says; a reference lists only its name. */
  kind?: SymbolKind;
  external: boolean;
}

const placeOf = ({ location, within }: Reference): Place => ({
  path: location.path,
  at: location.span.start,
  within,
  external: location.external,
});

const containerOfPlace = (p: Place): string | undefined => (p.within ? partsOf(p.within).container : undefined);
const isTestPlace = (p: Place): boolean => isTest(p.path, p.within ? containersOf(p.within.id) : []);

function groupBy<T>(items: readonly T[], keyOf: (item: T) => string): T[][] {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const key = keyOf(item);
    groups.set(key, [...(groups.get(key) ?? []), item]);
  }
  return [...groups.values()];
}

/** One row for the places in one member (or at the top level of a file), their lines together. */
function memberEntry(key: string, places: Place[], inGroup: boolean): Entry {
  const [first] = places;
  const lines = [...new Set(places.map((p) => lineText(p.at.line)))].join(" ");
  const common = {
    key,
    lines,
    count: 0,
    muted: isTestPlace(first),
    go: first.external ? undefined : open(first.path, first.at.line, "target"),
    sub: first.external ? "outside the repository" : undefined,
    children: [],
  };
  if (!first.within) {
    return {
      ...common,
      glyph: "file",
      name: fileName(first.path),
      file: true,
      title: rowTitle(null, first.path, first.at),
    };
  }
  const { name, container } = partsOf(first.within);
  return {
    ...common,
    glyph: first.kind ?? "other",
    name,
    // A free member names its file where a type's names the type.
    container: inGroup ? undefined : (container ?? fileName(first.path)),
    loc: inGroup ? undefined : locOf(first.path, container),
    title: rowTitle(first.within.id, first.path, first.at),
  };
}

/** The places by type (by file for a free function): a type with one member is that member's row, else a row that opens on its members. */
export function groupPlaces(places: readonly Place[], key: string): Entry[] {
  return groupBy(places, (p) => `${p.path}\0${containerOfPlace(p) ?? ""}`).map((group, g) => {
    const members = groupBy(group, (p) => (p.within ? partsOf(p.within).name : ""));
    const at = `${key}/${g}`;
    if (members.length === 1) return memberEntry(at, members[0], false);
    const [first] = group;
    const container = containerOfPlace(first);
    const rows = members.map((m, i) => memberEntry(`${at}/${i}`, m, true));
    return {
      key: at,
      glyph: container ? "type" : "file",
      name: container ?? fileName(first.path),
      file: !container,
      loc: container ? fileName(first.path) : undefined,
      muted: rows.every((r) => r.muted),
      count: rows.length,
      title: first.path,
      children: rows,
    };
  });
}

/** Places as rows: the production ones in groups, the tests in a branch of their own after them. */
function placesBody(places: readonly Place[], key: string): Pick<Body, "entries" | "tail"> {
  const entries = groupPlaces(
    places.filter((p) => !isTestPlace(p)),
    `${key}p`,
  );
  const tests = groupPlaces(places.filter(isTestPlace), `${key}t`);
  if (!tests.length) return { entries, tail: [] };
  const n = countLeaves(tests);
  const branch: Entry = {
    key: `${key}tests`,
    name: "Tests",
    branch: true,
    muted: true,
    count: n,
    title: `${n} ${n === 1 ? "place" : "places"} in tests`,
    children: tests,
  };
  return { entries, tail: [branch] };
}

export function affectedBody(affected: readonly SymbolInfo[], gaps: readonly string[]): Body {
  const places = affected.map((s): Place => ({ path: s.id.path, at: s.at, within: s, kind: s.kind, external: false }));
  const empty = gaps.length ? incomplete(gaps) : "No unchanged callers to check";
  return {
    count: affected.length,
    notes: gaps.length && affected.length ? [{ text: incomplete(gaps), warn: true }] : [],
    ...placesBody(places, "a"),
    quiet: affected.length ? undefined : empty,
  };
}

export function usagesBody(usages: Usages): Body {
  const notes = usages.gap ? [{ text: `References could not be listed: ${usages.gap}`, warn: true }] : [];
  return {
    count: usages.references.length,
    notes,
    ...placesBody(usages.references.map(placeOf), "u"),
    quiet: usages.references.length || notes.length ? undefined : "No usages",
  };
}

/** The path the backend finds runs from the symbol to the entry point; it is shown from the entry point down. */
export function reachBody(reach: Reach): Body {
  if (reach.type === "unreachedWithinBound") {
    return { notes: [], entries: [], tail: [], quiet: "Not reached from an entry point within the bounds" };
  }
  if (reach.type === "unknown") {
    const why = reach.gaps.length ? reach.gaps.join("; ") : "the analysis has gaps";
    return { notes: [], entries: [], tail: [], quiet: `Unknown: ${why}` };
  }
  // Each step wraps the ones before it, so the entry point ends up outermost.
  let below: Entry[] = [];
  reach.path.forEach((step, i) => {
    const { path } = step.id;
    below = [
      {
        key: `r${i}`,
        glyph: "function",
        name: shortLabel(step),
        loc: fileName(path),
        count: 0,
        fixed: true,
        title: rowTitle(step.id, path),
        go: open(path, undefined, "target"),
        children: below,
      },
    ];
  });
  const entry = reach.path.at(-1);
  return { meta: entry && shortLabel(entry), notes: [], entries: below, tail: [] };
}

/** A reference is one that needs attention unless it was deleted or edited along with the symbol. */
const needsAttention = (site: CallSite): boolean => site.fate.type !== "gone" && site.fate.type !== "modified";

function siteEntry(key: string, site: CallSite): Entry {
  const place = placeOf(site.base);
  const fate = describeFate(site.fate);
  const row = memberEntry(key, [place], false);
  const where = `${fileName(place.path)}${lineText(place.at.line)}`;
  return {
    ...row,
    // The second line names the file.
    container: place.within ? partsOf(place.within).container : undefined,
    loc: undefined,
    lines: undefined,
    fate: { label: fate.label, tone: fate.tone },
    sub: [place.external ? "outside the repository" : where, fate.detail].filter(Boolean).join(" · "),
    muted: !needsAttention(site),
    go: place.external ? undefined : open(place.path, place.at.line, "base"),
  };
}

/** What still refers to a removed symbol: what needs attention, open, then what is already handled. */
export function removalBody(removal: RemovalImpact): Body {
  const sites = removal.callSites.map((site, i) => ({ site, entry: siteEntry(`s${i}`, site) }));
  const branch = (key: string, name: string, rows: Entry[], o: Partial<Entry>): Entry[] =>
    rows.length ? [{ key, name, branch: true, count: rows.length, title: "", children: rows, ...o }] : [];
  const entries = [
    ...branch(
      "attention",
      "Needs attention",
      sites.filter((s) => needsAttention(s.site)).map((s) => s.entry),
      {
        open: true,
      },
    ),
    ...branch(
      "handled",
      "Already handled",
      sites.filter((s) => !needsAttention(s.site)).map((s) => s.entry),
      {
        muted: true,
      },
    ),
  ];
  const notes = removal.gap ? [{ text: `References could not be listed: ${removal.gap}`, warn: true }] : [];
  return {
    count: removal.callSites.length,
    notes,
    entries,
    tail: [],
    quiet: entries.length || notes.length ? undefined : "No remaining references",
  };
}
