// Pure logic of the Symbol panel: grouping the changed symbols by file, and the words
// shown for a symbol and for the fate of a call site.

import type { CallFate } from "../../generated/CallFate";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { ChangedSymbols } from "../../generated/ChangedSymbols";
import type { Position } from "../../generated/Position";
import type { Side } from "../../generated/Side";
import type { SymbolId } from "../../generated/SymbolId";
import { match, type Range } from "../../lib/speedSearch.ts";
import { keyOf } from "../../state/symbolKey.ts";

/** Opens a file at a 0-based `line` of `side`; no line opens it where it is. */
export type OpenLocation = (path: string, line: number | undefined, side: Side) => void;

export interface FileSymbols {
  path: string;
  symbols: ChangedSymbol[];
  /** Why there are no symbols to list: unknown, not unchanged. */
  note?: string;
}

/** The symbol's own name. */
export const nameOf = (id: SymbolId): string => id.name[id.name.length - 1] ?? id.path;

/** Its containers, outermost first, as `A::B`; empty at the top level of a file. */
export const containerOf = (id: SymbolId): string => id.name.slice(0, -1).join("::");

/** By file, then in source order; a removed symbol sits where it was on the base side. */
export function groupByFile(symbols: readonly ChangedSymbol[]): FileSymbols[] {
  const byPath = new Map<string, ChangedSymbol[]>();
  for (const s of symbols) {
    const list = byPath.get(s.symbol.id.path);
    if (list) list.push(s);
    else byPath.set(s.symbol.id.path, [s]);
  }
  return [...byPath]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([path, list]) => ({
      path,
      symbols: list.sort(
        (a, b) =>
          a.symbol.span.start.line - b.symbol.span.start.line || a.symbol.span.start.col - b.symbol.span.start.col,
      ),
    }));
}

/** The answer's files: those with symbols, then those whose symbols are unknown, with the reason. */
export function fileEntries(changed: ChangedSymbols): FileSymbols[] {
  const known = groupByFile(changed.symbols);
  const seen = new Set(known.map((g) => g.path));
  const unknown: FileSymbols[] = [];
  for (const gap of changed.gaps) {
    if (!seen.has(gap.path)) unknown.push({ path: gap.path, symbols: [], note: gap.reason });
  }
  return [...known, ...unknown.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))];
}

/**
 * The symbols whose name matches `query` under their files, files without one left out; `ranges`
 * has the matches to highlight.
 */
export function searchEntries(
  entries: readonly FileSymbols[],
  query: string,
): { entries: FileSymbols[]; ranges: Map<ChangedSymbol, Range[]> } {
  const ranges = new Map<ChangedSymbol, Range[]>();
  const found: FileSymbols[] = [];
  for (const entry of entries) {
    const symbols = entry.symbols.filter((s) => {
      const m = match(query, nameOf(s.symbol.id));
      if (m) ranges.set(s, m.ranges);
      return m;
    });
    if (symbols.length) found.push({ ...entry, symbols });
  }
  return { entries: found, ranges };
}

/**
 * The each-block keys of a file's symbol rows: the symbol's key and change, and for a symbol whose
 * id repeats (two object literals with the same property in one file) its count among those before.
 */
export function rowKeys(symbols: readonly ChangedSymbol[]): string[] {
  const seen = new Map<string, number>();
  return symbols.map((s) => {
    const key = keyOf(s.symbol.id) + s.change;
    const n = seen.get(key) ?? 0;
    seen.set(key, n + 1);
    return n ? `${key}#${n}` : key;
  });
}

/**
 * What a click on a changed symbol's row does, by the click's `detail`: a first click, or one from
 * the keyboard, selects the symbol; the second click of a double click shows it where it is
 * reviewed without asking about it again, and selects it only if the first click went elsewhere.
 */
export function rowClick(detail: number, selected: boolean): { select: boolean; reveal: boolean } {
  if (detail === 2) return { select: !selected, reveal: true };
  return { select: detail < 2, reveal: false };
}

/** `path:line` with a 1-based line, as editors print it. */
export const place = (path: string, at: Position): string => `${path}:${at.line + 1}`;

export type FateTone = "bad" | "warn" | "mute";

/** What became of a reference to a removed symbol, in words; `likelyBroken` is the one that needs attention. */
export function describeFate(fate: CallFate): { label: string; tone: FateTone; detail?: string } {
  switch (fate.type) {
    case "gone":
      return { label: "deleted", tone: "mute" };
    case "modified":
      return { label: "edited", tone: "mute" };
    case "likelyBroken":
      return { label: "likely broken", tone: "bad" };
    case "bindingChanged":
      return {
        label: "binds elsewhere now",
        tone: "warn",
        detail: fate.now.map((l) => place(l.path, l.span.start)).join(", ") || undefined,
      };
    case "unknown":
      return { label: "unknown", tone: "mute", detail: fate.reason };
  }
}
