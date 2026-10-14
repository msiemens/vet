// The rows of the Structure list: every symbol of a file, nested by the names of the symbols that hold
// them as the Changes tree nests the changed ones. Plain functions, so sorting, the filters and
// following the cursor are tested without a component.

import type { OutlineSymbol } from "../generated/OutlineSymbol";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import { collator } from "../lib/format.ts";
import { match, type Range } from "../lib/speedSearch.ts";
import { nameOf } from "../lib/symbolLabel.ts";
import { keyOf } from "./symbolKey.ts";

export interface StructureRow {
  kind: "symbol";
  /** Names the row among those of its file. */
  key: string;
  name: string;
  depth: number;
  symbol: SymbolInfo;
  /** Absent: the changeset did not change it, or the view has no marks. */
  change?: SymbolChange;
  /** Other symbols nest in it and are shown now. */
  holds: boolean;
  open: boolean;
  /** Listed only to place a match or a changed symbol: the row itself is neither. */
  muted: boolean;
  /** What the typed query matched in the name. */
  ranges?: Range[];
}

export interface RowOptions {
  /** The symbols under each holder alphabetically, else in the order of the file. */
  sort: boolean;
  /** Only the changed symbols, and the holders that place them. */
  changedOnly: boolean;
  /** Only the symbols whose names match, and the holders that place them; the holders open. */
  query: string;
  /** The keys of the holders that are shut. */
  closed: ReadonlySet<string>;
}

interface Entry {
  key: string;
  name: string;
  symbol: SymbolInfo;
  change?: SymbolChange;
  parent?: Entry;
  children: Entry[];
}

/** The outline as a forest: a symbol is nested in the longest proper prefix of its name that another has. */
function link(symbols: readonly OutlineSymbol[]): { entries: Entry[]; roots: Entry[] } {
  const seen = new Map<string, number>();
  const entries: Entry[] = symbols.map(({ symbol, change }) => {
    const base = keyOf(symbol.id);
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return {
      key: n ? `${base}#${n}` : base,
      name: nameOf(symbol.id),
      symbol,
      change: change ?? undefined,
      children: [],
    };
  });
  const byName = new Map<string, Entry>();
  for (const e of entries) {
    const name = JSON.stringify(e.symbol.id.name);
    if (!byName.has(name)) byName.set(name, e);
  }
  const roots: Entry[] = [];
  for (const e of entries) {
    const { name } = e.symbol.id;
    let holder: Entry | undefined;
    for (let length = name.length - 1; length > 0 && !holder; length--)
      holder = byName.get(JSON.stringify(name.slice(0, length)));
    e.parent = holder;
    (holder ? holder.children : roots).push(e);
  }
  return { entries, roots };
}

const names = collator({ sensitivity: "base" });

/** The visible rows, with the unfiltered ones in the order the backend listed them. */
export function structureRows(symbols: readonly OutlineSymbol[], o: RowOptions): StructureRow[] {
  const { roots } = link(symbols);
  const hits = new Map<Entry, Range[]>();
  const wanted = (e: Entry): boolean => {
    if (o.changedOnly && e.change === undefined) return false;
    if (!o.query) return true;
    const found = match(o.query, e.name);
    if (found) hits.set(e, found.ranges);
    return !!found;
  };
  // An entry is listed if it is wanted or holds one that is.
  const listed = new Map<Entry, boolean>();
  const lists = (e: Entry): boolean => {
    let yes = wanted(e);
    for (const child of e.children) yes = lists(child) || yes;
    listed.set(e, yes);
    return yes;
  };
  roots.forEach(lists);

  const rows: StructureRow[] = [];
  const walk = (level: Entry[], depth: number) => {
    const ordered = o.sort ? [...level].sort((a, b) => names.compare(a.name, b.name)) : level;
    for (const e of ordered) {
      if (!listed.get(e)) continue;
      const shown = e.children.filter((c) => listed.get(c));
      const open = !!o.query || !o.closed.has(e.key);
      rows.push({
        kind: "symbol",
        key: e.key,
        name: e.name,
        depth,
        symbol: e.symbol,
        change: e.change,
        holds: shown.length > 0,
        open,
        muted: (o.changedOnly && e.change === undefined) || (!!o.query && !hits.has(e)),
        ranges: hits.get(e),
      });
      if (open) walk(shown, depth + 1);
    }
  };
  walk(roots, 0);
  return rows;
}

/** The keys of the symbols that hold others. */
export function holderKeys(symbols: readonly OutlineSymbol[]): string[] {
  return link(symbols)
    .entries.filter((e) => e.children.length > 0)
    .map((e) => e.key);
}

/** The keys of the holders around the symbol `id`, so that opening them shows it. */
export function holdersOf(symbols: readonly OutlineSymbol[], id: string): string[] {
  const found = link(symbols).entries.find((e) => keyOf(e.symbol.id) === id);
  const keys: string[] = [];
  for (let e = found?.parent; e; e = e.parent) keys.push(e.key);
  return keys;
}

interface Place {
  line: number;
  col: number;
}
const compare = (a: Place, b: Place) => a.line - b.line || a.col - b.col;

/**
 * The innermost symbol of the target side that holds the position, as `keyOf` names it: a removed
 * symbol has no place in the target's text. A symbol is innermost if it starts last, and of two
 * that start together, if it ends first.
 */
export function enclosingId(symbols: readonly OutlineSymbol[], at: Place): string | undefined {
  let best: SymbolInfo | undefined;
  for (const { symbol, change } of symbols) {
    if (change === "removed") continue;
    const { start, end } = symbol.span;
    if (compare(start, at) > 0 || compare(at, end) > 0) continue;
    const later = best ? compare(start, best.span.start) : 1;
    if (!best || later > 0 || (later === 0 && compare(end, best.span.end) < 0)) best = symbol;
  }
  return best && keyOf(best.id);
}
