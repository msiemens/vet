// How the changed symbols of a revision are loaded and kept: when a load is needed, in what order
// the files are analysed, and what becomes of the symbol on show when the symbols are found again.

import { isAbort } from "../lib/abort.ts";
import { keyOf } from "./symbolKey.ts";
import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";

/** What was loaded: the connection and the revision of the changeset. */
export interface Loaded {
  backend: unknown;
  revision: number;
}

/**
 * What a request to load `next` comes to: nothing if it is what was loaded (the manifest is applied
 * again without its revision moving), the same analysis again if `force` (the connection or the
 * servers changed what an answer may be), else a load of a revision not seen.
 */
export function planLoad(prev: Loaded | undefined, next: Loaded, force: boolean): "skip" | "again" | "new" {
  if (prev && prev.backend === next.backend && prev.revision === next.revision) return force ? "again" : "skip";
  return "new";
}

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

export type Outcome<R> = { ok: true; value: R } | { ok: false; error: unknown };

export interface AnalyseOptions<R> {
  paths: readonly string[];
  /** The file the review shows: asked first, whenever a file is next. */
  current: () => string | undefined;
  /** Files asked at once. */
  atOnce: number;
  ask: (path: string, signal: AbortSignal) => Promise<R>;
  /** A file's outcome, unless `signal` aborted meanwhile. */
  done: (path: string, outcome: Outcome<R>) => void;
  signal: AbortSignal;
}

/** Asks for every file, a few at a time, until `signal` aborts. */
export function analyseFiles<R>(o: AnalyseOptions<R>): void {
  const queue = [...o.paths];
  const next = () => {
    const current = o.current();
    const first = current === undefined ? -1 : queue.indexOf(current);
    return first < 0 ? queue.shift() : queue.splice(first, 1)[0];
  };
  const aborted = () => o.signal.aborted;
  const worker = async () => {
    for (let path = next(); path !== undefined; path = next()) {
      if (aborted()) return;
      let outcome: Outcome<R>;
      try {
        outcome = { ok: true, value: await o.ask(path, o.signal) };
      } catch (error) {
        // A request that was cancelled is no outcome.
        if (isAbort(error)) return;
        outcome = { ok: false, error };
      }
      if (aborted()) return;
      o.done(path, outcome);
    }
  };
  for (let i = 0; i < Math.min(o.atOnce, o.paths.length); i++) void worker();
}
