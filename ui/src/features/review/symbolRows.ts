// The rows of a file's changed symbols in the Changes tree: the symbols nest by the names of the
// symbols that hold them (a method under its class, a function under its module), and a holder
// the change left alone is a row of its own, shown muted, only to say where the change is.

import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { SymbolChange } from "../../generated/SymbolChange";
import type { SymbolInfo } from "../../generated/SymbolInfo";
import { rowLabel } from "../../lib/symbolLabel.ts";
import { keyOf } from "../../state/symbolKey.ts";

export interface SymbolRow {
  kind: "symbol";
  /** The file the symbol is shown under. */
  path: string;
  /** Names the row among those of its file. */
  key: string;
  name: string;
  depth: number;
  symbol: SymbolInfo;
  /** Absent: a holder of changed symbols that the change left alone. */
  change?: SymbolChange;
  /** Other symbols nest in it, whether or not they are shown now. */
  holds: boolean;
}

interface Entry {
  symbol: SymbolInfo;
  change?: SymbolChange;
  key: string;
  children: Entry[];
}

const byPlace = (a: Entry, b: Entry) =>
  a.symbol.span.start.line - b.symbol.span.start.line || a.symbol.span.start.col - b.symbol.span.start.col;

/** The key of a symbol's row in its file's rows: closed holders are remembered by it. */
export const closedKey = (path: string, key: string): string => `${path}\0${key}`;

/**
 * The visible rows of the symbols of the file at `path`, in source order, nested under their
 * holders; what is in a holder of `closed` is left out. The first level is at `depth`.
 */
export function fileSymbolRows(
  path: string,
  changed: readonly ChangedSymbol[],
  context: readonly SymbolInfo[],
  depth: number,
  closed: ReadonlySet<string>,
): SymbolRow[] {
  const seen = new Map<string, number>();
  const entries: Entry[] = [
    ...changed.map((c) => ({ symbol: c.symbol, change: c.change })),
    ...context.map((symbol) => ({ symbol, change: undefined })),
  ].map((e) => {
    const base = keyOf(e.symbol.id) + (e.change ?? "");
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return { ...e, key: n ? `${base}#${n}` : base, children: [] };
  });

  // The holder of a symbol is the longest proper prefix of its name that another entry has.
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
    (holder ? holder.children : roots).push(e);
  }

  const rows: SymbolRow[] = [];
  const walk = (level: Entry[], at: number) => {
    for (const e of level.sort(byPlace)) {
      rows.push({
        kind: "symbol",
        path,
        key: e.key,
        name: rowLabel(e.symbol.id, path).name,
        depth: at,
        symbol: e.symbol,
        change: e.change,
        holds: e.children.length > 0,
      });
      if (!closed.has(closedKey(path, e.key))) walk(e.children, at + 1);
    }
  };
  walk(roots, depth);
  return rows;
}

/** What a row has to do with ← and →: whether it can be open or shut, and whether it is open. */
export interface Foldable {
  kind: string;
  depth: number;
}

/**
 * What ← or → does on the row at `index`: opens or shuts it, or moves the highlight, to the first
 * row inside it or to the row that holds it. `holds` says whether a row can be open or shut and
 * `open` whether it is.
 */
export function arrowKey<R extends Foldable>(
  rows: readonly R[],
  index: number,
  key: "left" | "right",
  holds: (row: R) => boolean,
  open: (row: R) => boolean,
): { toggle: R } | { move: number } | undefined {
  if (index < 0 || index >= rows.length) return undefined;
  const row = rows[index];
  if (key === "right") {
    if (!holds(row)) return undefined;
    if (!open(row)) return { toggle: row };
    return index + 1 < rows.length && rows[index + 1].depth > row.depth ? { move: index + 1 } : undefined;
  }
  if (holds(row) && open(row)) return { toggle: row };
  if (row.kind !== "symbol") return undefined;
  for (let at = index - 1; at >= 0; at--) if (rows[at].depth < row.depth) return { move: at };
  return undefined;
}
