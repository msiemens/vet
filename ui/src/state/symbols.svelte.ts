// The Symbol panel's state: the changeset's changed symbols, the selected one and the
// answers about it. The backend derives them (SPEC §7.3, I5); this loads and holds them,
// each one on its own, since an impact query may take a server's cold start. The changed
// symbols belong to a revision of the open repository: the store follows both.

import { untrack } from "svelte";
import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { ChangedSymbols } from "../generated/ChangedSymbols";
import type { Impact } from "../generated/Impact";
import type { Reachability } from "../generated/Reachability";
import type { RemovalImpact } from "../generated/RemovalImpact";
import type { SymbolSignature } from "../generated/SymbolSignature";
import type { Unavailable } from "../generated/Unavailable";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";
import { session } from "./session.svelte";
import { keyOf } from "./symbolKey";

export type Load<T> = { state: "loading" } | { state: "done"; value: T } | { state: "failed"; message: string };

/** The signature the backend found; `unavailable` says why there is none. */
export interface Signature {
  signature?: SymbolSignature;
  unavailable?: Unavailable;
}

class SymbolsState {
  changed = $state.raw<Load<ChangedSymbols>>();
  selected = $state.raw<ChangedSymbol>();
  /** The file the review shows; the list scrolls to it. */
  file = $state<string>();
  /** Only the current file's symbols are listed. */
  currentOnly = $state(false);
  // The selected symbol's answers: impact and reachability for an added or modified one,
  // removal impact for a removed one.
  signature = $state.raw<Load<Signature>>();
  impact = $state.raw<Load<Impact>>();
  reach = $state.raw<Load<Reachability>>();
  removal = $state.raw<Load<RemovalImpact>>();

  private backend?: Backend;
  private revision?: number;
  // A reply of an older request is dropped.
  private listRun = 0;
  private detailRun = 0;

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const { backend, repo } = session;
        const revision = changeset.manifest?.revision;
        untrack(() => {
          if (repo === undefined) this.reset();
          else if (backend && revision !== undefined) void this.load(backend, revision);
        });
      });
    });
  }

  /** Back to nothing: no repository is open. */
  private reset() {
    this.listRun++;
    this.backend = this.revision = undefined;
    this.changed = undefined;
    this.clearSelection();
  }

  /** Loads the changed symbols of `revision`, keeping the selection if its symbol is still changed. */
  private async load(backend: Backend, revision: number) {
    if (this.backend === backend && this.revision === revision) return;
    this.backend = backend;
    this.revision = revision;
    const run = ++this.listRun;
    this.changed = { state: "loading" };
    let loaded: Load<ChangedSymbols>;
    try {
      loaded = { state: "done", value: await backend.changedSymbols(revision) };
    } catch (error) {
      loaded = { state: "failed", message: errorText(error) };
    }
    if (run !== this.listRun) return;
    this.changed = loaded;
    const previous = this.selected && keyOf(this.selected.symbol.id);
    const kept =
      loaded.state === "done" ? loaded.value.symbols.find((s) => keyOf(s.symbol.id) === previous) : undefined;
    if (kept) this.select(kept);
    else this.clearSelection();
  }

  select(symbol: ChangedSymbol) {
    const { backend, revision } = this;
    if (!backend || revision === undefined) return;
    const run = ++this.detailRun;
    this.selected = symbol;
    const { id } = symbol.symbol;
    this.impact = this.reach = this.removal = undefined;
    this.signature = this.start(
      run,
      () => backend.signature(revision, id),
      (l) => (this.signature = l),
    );
    if (symbol.change === "removed") {
      this.removal = this.start(
        run,
        () => backend.removalImpact(revision, id),
        (l) => (this.removal = l),
      );
    } else {
      this.impact = this.start(
        run,
        () => backend.impact(revision, id),
        (l) => (this.impact = l),
      );
      this.reach = this.start(
        run,
        () => backend.reachability(revision, id),
        (l) => (this.reach = l),
      );
    }
  }

  clearSelection() {
    this.detailRun++;
    this.selected = this.signature = this.impact = this.reach = this.removal = undefined;
  }

  /** Runs `query`, hands the settled state to `set` unless another selection came first; returns the pending state. */
  private start<T>(run: number, query: () => Promise<T>, set: (load: Load<T>) => void): Load<T> {
    query().then(
      (value) => run === this.detailRun && set({ state: "done", value }),
      (error: unknown) => run === this.detailRun && set({ state: "failed", message: errorText(error) }),
    );
    return { state: "loading" };
  }
}

export const symbols = new SymbolsState();
