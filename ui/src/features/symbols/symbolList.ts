// Pure logic of the Symbol panel: grouping the changed symbols by file, finding the one
// at a position, and the words shown for a symbol and for the fate of a call site.

import type { CallFate } from "../../generated/CallFate";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { ChangedSymbols } from "../../generated/ChangedSymbols";
import type { Position } from "../../generated/Position";
import type { Side } from "../../generated/Side";
import type { SymbolId } from "../../generated/SymbolId";
import type { TextSpan } from "../../generated/TextSpan";

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

const before = (a: Position, b: Position) => a.line < b.line || (a.line === b.line && a.col < b.col);
const contains = (span: TextSpan, at: Position) => !before(at, span.start) && before(at, span.end);

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
  for (const path of changed.unsupported) {
    if (!seen.has(path)) unknown.push({ path, symbols: [], note: "Language unsupported" });
  }
  for (const gap of changed.gaps) {
    if (!seen.has(gap.path)) unknown.push({ path: gap.path, symbols: [], note: gap.reason });
  }
  return [...known, ...unknown.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))];
}

/**
 * The innermost changed symbol whose definition contains the position of `side`. A removed
 * symbol exists on the base side only, the others on the target side only.
 */
export function symbolAt(
  symbols: readonly ChangedSymbol[],
  path: string,
  side: Side,
  line: number,
  col: number,
): ChangedSymbol | undefined {
  const at = { line, col };
  let best: ChangedSymbol | undefined;
  for (const s of symbols) {
    if (s.symbol.id.path !== path || (s.change === "removed") !== (side === "base")) continue;
    if (!contains(s.symbol.span, at)) continue;
    // Nested definitions are inside their container: the deeper chain is the innermost.
    if (!best || s.symbol.id.name.length > best.symbol.id.name.length) best = s;
  }
  return best;
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
