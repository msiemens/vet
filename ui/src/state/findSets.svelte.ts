// The Find tool's result sets: what Find in Files and Find usages found, kept as tabs. A new result
// goes to the tab that is not pinned, replacing what it held; a pinned tab is never replaced, and the
// same query asked again reruns its tab. Sets live for the session of a changeset.

import { SvelteMap, SvelteSet } from "svelte/reactivity";
import type { MappedLine } from "../generated/MappedLine";
import type { LocationRows } from "../generated/LocationRows";
import type { Side } from "../generated/Side";
import { isAbort } from "../lib/abort";
import { errorText } from "../lib/errorText";
import type { SearchQuery, SearchResults } from "./backend";
import { changeset } from "./changeset.svelte";
import { type ChangedFile, type Place, placesFromRows, placesFromSearch } from "./foundPlace";
import { type Filters, NO_FILTERS } from "./findRows";
import { changedSince, diffRuns, isStale, linesToLocate, ranVersion, slotFor } from "./findRuns";
import { session } from "./session.svelte";
import { versions } from "./versions.svelte";

/** Find usages of the symbol at a position of one side's text. */
export interface UsagesSpec {
  type: "usages";
  /** The name the tab carries. */
  symbol: string;
  side: Side;
  path: string;
  /** 0-based. */
  line: number;
  col: number;
}

/** How Find in Files was asked, which the find bar of the view a result opens in carries on. */
export interface SearchOptions {
  text: string;
  matchCase: boolean;
  wholeWord: boolean;
  regex: boolean;
  side: "both" | "target" | "base";
  onlyChanges: boolean;
}

/** A search of Find in Files, run again as it was asked. */
export interface SearchSpec {
  type: "search";
  request: SearchQuery;
  options: SearchOptions;
  /** What the places' line numbers are of: the place searched. */
  scope: { label: string; commit?: { id: string; shortId: string } };
}

export type SetSpec = UsagesSpec | SearchSpec;

/** A search of the dialog with its results, handed over by "Open in Find window". */
export interface KeptSearch {
  spec: SearchSpec;
  results: SearchResults;
}

/** Only the query, not how many matches it was asked to stop at. */
const keyOf = (spec: SetSpec): string =>
  spec.type === "usages"
    ? `u|${spec.side}|${spec.path}|${spec.line}:${spec.col}`
    : `s|${JSON.stringify({ ...spec.request, limit: 0 })}`;

/** The most places Find usages asks for when the whole list is wanted. */
const NO_LIMIT = 1_000_000;

export class FindSet {
  readonly spec: SetSpec;
  readonly key: string;
  pinned = $state(false);
  phase = $state<"running" | "done" | "stopped" | "failed">("running");
  places = $state.raw<readonly Place[]>([]);
  /** How many places there are where the answer was cut at fewer. */
  total = $state(0);
  /** Why the set has no places: a language server that cannot answer, or a failed request. */
  message = $state<string>();
  /** The revision and the version the places are of. */
  ran = $state.raw<{ revision?: number; version?: number }>({});
  /** What the last rerun changed. */
  rerun = $state.raw<{ fresh: number; gone: number }>();
  fresh = $state.raw<ReadonlySet<string>>(new SvelteSet());
  filters = $state<Filters>({ ...NO_FILTERS });
  folded = $state.raw<ReadonlySet<string>>(new SvelteSet());
  selected = $state<string>();
  /** A search stopped at its limit. */
  truncated = $state(false);
  limit?: number;
  abort?: AbortController;
  hasRun = $state(false);
  /** Where the places of the files changed since the run are now, by place id. */
  moves = $state.raw<ReadonlyMap<string, MappedLine>>(new SvelteMap());
  /** The revision `moves` was asked for. */
  located?: number;

  constructor(
    readonly id: string,
    spec: SetSpec,
  ) {
    this.spec = spec;
    this.key = keyOf(spec);
  }

  /** What the tab is called: the symbol or the text searched for. */
  get title(): string {
    return this.spec.type === "usages" ? this.spec.symbol : this.spec.options.text;
  }

  /** The set's place is a commit's text, which nothing can make stale. */
  get commit(): boolean {
    return this.spec.type === "search" && this.spec.scope.commit !== undefined;
  }
}

class FindSets {
  sets = $state.raw<readonly FindSet[]>([]);
  active = $state<string>();
  preview = $state(true);
  /** Shows the Find tool, and puts the focus in its list if `focus`; set by the window that owns the bottom panel. */
  show: (focus: boolean) => void = () => undefined;
  private seq = 0;

  get current(): FindSet | undefined {
    return this.sets.find((s) => s.id === this.active);
  }

  /** The revision on show, which a set is stale against. */
  get revision(): number | undefined {
    return changeset.layout?.shown.revision;
  }

  /** The version a run of `spec` is of, which the set records and the stale banner names files since. */
  private versionOf(spec: SetSpec): number | undefined {
    return ranVersion(spec.type === "search" ? spec.request.scope : undefined, versions.items.at(-1)?.n);
  }

  /** The files versions after the one `set` ran on changed. */
  changedFiles(set: FindSet): ReadonlySet<string> {
    return changedSince(versions.items, set.ran.version);
  }

  stale(set: FindSet): boolean {
    return isStale({ done: set.phase === "done", commit: set.commit, revision: set.ran.revision }, this.revision);
  }

  /**
   * Asks where the places of the files changed since `set` ran are now, once for each revision, so
   * a row opens at its line as it is and the list can say which lines an edit replaced.
   */
  async locate(set: FindSet) {
    const backend = session.backend;
    const from = set.ran.version;
    const revision = this.revision;
    if (!backend || from === undefined || revision === undefined || set.located === revision || !this.stale(set))
      return;
    set.located = revision;
    const moves = new SvelteMap<string, MappedLine>();
    for (const path of this.changedFiles(set)) {
      const lines = linesToLocate(set.places, path);
      if (!lines.length) continue;
      try {
        const mapped = await backend.mapLines(revision, from, path, lines);
        if (set.located !== revision) return;
        for (const p of set.places) {
          const found = p.file.path === path && p.side === "target" ? mapped[lines.indexOf(p.line)] : undefined;
          if (found) moves.set(p.id, found);
        }
      } catch {
        // A file with no text then or now keeps the lines it had.
      }
    }
    if (set.located === revision) set.moves = moves;
  }

  /** Finds the usages at a position, into the tool. */
  findUsages(spec: UsagesSpec) {
    void this.start(this.place(spec));
    this.show(true);
  }

  /** Takes the list a Show Usages popup holds into the tool, as it was found. */
  keepUsages(spec: UsagesSpec, answer: LocationRows) {
    const set = this.place(spec);
    this.settle(set, () => {
      set.places = placesFromRows(answer.rows, answer.side, this.changedFile);
      set.total = answer.total;
    });
    this.show(false);
  }

  /** Takes the results of the Find in Files dialog into the tool. */
  keepSearch({ spec, results }: KeptSearch) {
    const set = this.place(spec);
    this.settle(set, () => this.takeSearch(set, results));
    this.show(false);
  }

  /** The tab a result of `spec` goes to, made the active one. */
  private place(spec: SetSpec): FindSet {
    const fresh = new FindSet(`f${++this.seq}`, spec);
    const slot = slotFor(this.sets, fresh.key);
    let set = fresh;
    if (slot.kind === "rerun") set = this.sets[slot.index];
    else if (slot.kind === "replace") {
      this.sets[slot.index].abort?.abort();
      this.sets = this.sets.map((s, i) => (i === slot.index ? fresh : s));
    } else this.sets = [...this.sets, fresh];
    this.active = set.id;
    return set;
  }

  /** What the changeset on show says of a changed file, if its view is the changeset's. */
  private changedFile = (path: string): ChangedFile | undefined => {
    if (changeset.view.type !== "changeset") return undefined;
    const file = changeset.reviewed(path);
    return file && { status: file.status, completion: file.completion, hunks: file.hunks, states: file.review.hunks };
  };

  private settle(set: FindSet, take: () => void) {
    const before = set.hasRun ? set.places : undefined;
    set.abort?.abort();
    take();
    this.finish(set, before);
  }

  private finish(set: FindSet, before?: readonly Place[]) {
    set.phase = "done";
    set.message = undefined;
    set.hasRun = true;
    set.ran = { revision: this.revision, version: this.versionOf(set.spec) };
    const { fresh, gone } = before ? diffRuns(before, set.places) : { fresh: new SvelteSet<string>(), gone: 0 };
    set.fresh = fresh;
    set.moves = new SvelteMap();
    set.located = undefined;
    set.rerun = before ? { fresh: fresh.size, gone } : undefined;
    if (!set.places.some((p) => p.id === set.selected)) set.selected = undefined;
    if (set.folded.size) set.folded = new SvelteSet();
  }

  private takeSearch(set: FindSet, results: SearchResults) {
    set.places = placesFromSearch(results.files);
    set.total = results.hits;
    set.truncated = results.truncated;
  }

  /** Runs the set: the places of the run before stay until this one answers. */
  async start(set: FindSet) {
    const backend = session.backend;
    if (!backend) return;
    set.abort?.abort();
    const mine = new AbortController();
    set.abort = mine;
    set.phase = "running";
    const before = set.hasRun ? set.places : undefined;
    const { spec } = set;
    try {
      if (spec.type === "usages") {
        const revision = this.revision;
        if (revision === undefined) throw new Error("No changeset is on view");
        const answer = await backend.usages(
          revision,
          spec.side,
          spec.path,
          spec.line,
          spec.col,
          mine.signal,
          set.limit,
        );
        if (mine.signal.aborted) return;
        if (answer.unavailable) {
          set.places = [];
          set.total = 0;
          set.phase = "stopped";
          set.message = answer.unavailable.message;
          set.hasRun = true;
          return;
        }
        set.places = placesFromRows(answer.rows, answer.side, this.changedFile);
        set.total = answer.total;
      } else {
        const results = await backend.search({ ...spec.request, limit: set.limit ?? spec.request.limit }, mine.signal);
        if (mine.signal.aborted) return;
        this.takeSearch(set, results);
      }
      this.finish(set, before);
    } catch (error) {
      if (mine.signal.aborted) return;
      set.phase = "failed";
      set.message = isAbort(error) ? "The changeset changed while this ran. Rerun it." : errorText(error);
    }
  }

  rerun(set: FindSet) {
    void this.start(set);
  }

  /** Reads the places of a usages list that was cut, all of them. */
  showAll(set: FindSet) {
    set.limit = NO_LIMIT;
    void this.start(set);
  }

  stop(set: FindSet) {
    set.abort?.abort();
    set.phase = set.hasRun ? "done" : "stopped";
    if (!set.hasRun) set.message = "Stopped";
  }

  close(id: string) {
    const at = this.sets.findIndex((s) => s.id === id);
    if (at < 0) return;
    this.sets[at].abort?.abort();
    const rest = this.sets.filter((s) => s.id !== id);
    this.sets = rest;
    if (this.active === id) this.active = (rest.at(at) ?? rest.at(at - 1))?.id;
  }

  togglePin(set: FindSet) {
    set.pinned = !set.pinned;
  }

  /** Forgets every set: they belong to the changeset that was on view. */
  reset() {
    for (const s of this.sets) s.abort?.abort();
    this.sets = [];
    this.active = undefined;
  }
}

export const findSets = new FindSets();
