// What becomes of the symbol on show when the analysis of its file arrives again.

import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import { keyOf } from "./symbolKey.ts";

/** A symbol on show, and what the changeset did to it. */
export interface Shown {
  symbol: SymbolInfo;
  /** Absent: the changeset did not change it. */
  change?: SymbolChange;
}

/**
 * The symbol on show once its file's symbols are found again: as the analysis has it if it is still
 * a changed one, else unchanged.
 */
export function reconcile(shown: Shown, found: { path: string; symbols: readonly ChangedSymbol[] }): Shown {
  const key = keyOf(shown.symbol.id);
  const hit = found.symbols.find((c) => keyOf(c.symbol.id) === key);
  if (hit) return { symbol: hit.symbol, change: hit.change };
  return shown.symbol.id.path === found.path && shown.change !== undefined
    ? { symbol: shown.symbol, change: undefined }
    : shown;
}
