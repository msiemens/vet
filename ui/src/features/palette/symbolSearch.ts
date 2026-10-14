// The search of the language servers for symbols by name, as typing drives it: each search cancels
// the one before, and an answer that arrives after a newer search began counts for nothing.

import type { LanguageSymbols } from "../../generated/LanguageSymbols";
import { isAbort } from "../../lib/abort.ts";
import { Resource } from "../../state/resource.ts";

/** What the servers found for a query on a revision of the changeset. */
export interface Found {
  query: string;
  revision: number;
  languages: LanguageSymbols[];
}

export interface SearchEffects {
  /** A search is under way (`true`), or none is any more. */
  busy?(on: boolean): void;
  /** The latest search's answer. */
  apply(found: Found): void;
  /** The latest search failed for another reason than being cancelled. */
  fail(query: string, revision: number, error: unknown): void;
}

export class SymbolSearch {
  private readonly loads: Resource<Found>;
  private readonly ask: (query: string, revision: number, signal: AbortSignal) => Promise<LanguageSymbols[]>;
  private readonly effects: SearchEffects;
  private abort?: AbortController;

  constructor(
    ask: (query: string, revision: number, signal: AbortSignal) => Promise<LanguageSymbols[]>,
    effects: SearchEffects,
  ) {
    this.ask = ask;
    this.effects = effects;
    this.loads = new Resource<Found>({ busy: (on) => effects.busy?.(on), apply: (found) => effects.apply(found) });
  }

  /** Asks the servers; the search under way is cancelled, down to the language server. */
  search(query: string, revision: number) {
    this.abort?.abort();
    const { signal } = (this.abort = new AbortController());
    this.loads
      .load(async () => ({ query, revision, languages: await this.ask(query, revision, signal) }))
      .catch((error: unknown) => {
        if (!isAbort(error)) this.effects.fail(query, revision, error);
      });
  }

  /** Stops the search under way; its answer, if one still comes, is dropped. */
  cancel() {
    this.abort?.abort();
    this.abort = undefined;
    this.loads.cancel();
  }
}
