// The Symbol tool's state: the changed symbols of each changed file, which the Changes tree shows
// under their files, and the symbol on show, with the answers about it. The backend derives them
// (SPEC §7.3, I5); this loads and holds them. The symbols belong to a revision of the open
// repository, and each file's arrive as its analysis ends. The symbol on show is the one the
// reviewer chose last: a row of the tree, or the symbol the cursor of an editor is in.

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { Calls } from "../generated/Calls";
import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { Reachability } from "../generated/Reachability";
import type { RemovalImpact } from "../generated/RemovalImpact";
import type { Side } from "../generated/Side";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import type { SymbolSignature } from "../generated/SymbolSignature";
import type { Unavailable } from "../generated/Unavailable";
import type { Usages } from "../generated/Usages";
import { isAbort } from "../lib/abort";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";
import { CursorFollow } from "./cursorFollow";
import { reviewShown } from "./reviewShown.svelte";
import { session } from "./session.svelte";
import { keyOf } from "./symbolKey";
import { analyseFiles, isGone, planLoad, reconcile, type Shown } from "./symbolLoad";

export type Load<T> = { state: "loading" } | { state: "done"; value: T } | { state: "failed"; message: string };

/** The signature the backend found; `unavailable` says why there is none. */
export interface Signature {
  signature?: SymbolSignature;
  unavailable?: Unavailable;
}

/** What the analysis of one changed file found. */
export type FileLoad =
  | { state: "loading" }
  | { state: "done"; symbols: ChangedSymbol[]; context: SymbolInfo[] }
  | { state: "failed"; message: string };

/** Files analysed at once. */
const FILES_AT_ONCE = 4;
/** How long the cursor rests before the symbol it is in is looked up. */
const CURSOR_MS = 150;

/** Where an editor's cursor is. */
interface CursorPlace {
  revision: number;
  side: Side;
  path: string;
  line: number;
  col: number;
}

class SymbolsState {
  /** The analysis of each changed file with text hunks, as of the revision loaded. */
  files = new SvelteMap<string, FileLoad>();
  /** The files of the revision whose analysis ended, of those it asks for. */
  progress = $state.raw({ done: 0, total: 0 });
  /** The file the review shows; its analysis comes first. */
  file = $state<string>();
  selected = $state.raw<Shown>();
  /** Why the cursor's symbol is unknown, when the language has no usable server. */
  unavailable = $state.raw<Unavailable>();
  /** Asks the tree to show the selected changed symbol under its file `path`; `n` makes a repeated ask a new one. */
  reveal = $state.raw<{ symbol: SymbolInfo; path: string; n: number }>();
  // The selected symbol's answers: the target side's calls and usages and its reach, or a removed
  // symbol's remaining references.
  signature = $state.raw<Load<Signature>>();
  calls = $state.raw<Load<Calls>>();
  usages = $state.raw<Load<Usages>>();
  reach = $state.raw<Load<Reachability>>();
  removal = $state.raw<Load<RemovalImpact>>();

  /** Where the cursor moved while the review screen was away; followed when it comes up. */
  private held?: CursorPlace;
  /** The analysis has to be asked again when the review screen comes up: the servers changed while it was away. */
  private reloadDue = false;
  private backend?: Backend;
  private revision?: number;
  private paths: readonly string[] = [];
  // Aborts the analysis of the files of an older load.
  private loading?: AbortController;
  // The requests about the selected symbol; aborted when the selection moves.
  private asking?: AbortController;
  private detailRun = 0;
  private revealed = 0;
  private cursor = new CursorFollow<CursorPlace, Awaited<ReturnType<Backend["symbolAt"]>>>({
    ms: CURSOR_MS,
    key: (p) => JSON.stringify([p.revision, p.side, p.path, p.line, p.col]),
    ask: (p, signal) => this.requireBackend().symbolAt(p.revision, p.side, p.path, p.line, p.col, signal),
    apply: (answer) => {
      if (answer.symbol) {
        this.followed({ symbol: answer.symbol, change: answer.change ?? undefined });
      } else {
        this.clearSelection();
        this.unavailable = answer.unavailable ?? undefined;
      }
    },
    fail: () => {
      // The next move asks again; what is on show stays.
    },
  });

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const { backend, repo } = session;
        const layout = changeset.layout;
        // The semantic layer has the changeset's snapshots: not a commit's, and not a range's.
        const analysable = changeset.shownCommit === undefined && layout?.interdiff === undefined;
        const revision = layout?.shown.revision;
        const paths = layout?.files.filter((f) => f.hunks.length > 0).map((f) => f.path);
        // The analysis is for the review screen: the files wait until it is shown.
        const shown = reviewShown.on;
        untrack(() => {
          if (repo === undefined || revision === undefined || !analysable || !paths) this.reset();
          else if (backend && shown) this.load(backend, revision, paths, this.reloadDue);
        });
      });
    });
    reviewShown.onShown(() => {
      const place = this.held;
      this.held = undefined;
      if (place) this.cursor.move(place);
    });
    // What was asked while the connection was down may have failed.
    session.onResync((backend) => {
      if (this.backend === backend) this.reload();
    });
    // A server installed or reconfigured may answer for files it had no answer for.
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.type !== "serversChanged" || event.project !== backend.project) return;
        if (this.backend === backend) this.reload();
      }),
    );
  }

  private requireBackend(): Backend {
    if (!this.backend) throw new Error("no repository is open");
    return this.backend;
  }

  /** Back to nothing: no repository is open, or no changeset to analyse is on view. */
  private reset() {
    this.loading?.abort();
    this.backend = this.revision = undefined;
    this.paths = [];
    this.files.clear();
    this.progress = { done: 0, total: 0 };
    this.cursor.reset();
    this.held = undefined;
    this.clearSelection();
  }

  private reload() {
    if (!this.backend || this.revision === undefined) return;
    if (reviewShown.on) this.load(this.backend, this.revision, this.paths, true);
    else this.reloadDue = true;
  }

  /**
   * Analyses the files of `revision`, a few at a time, the file the review shows first. The files of
   * an earlier load stay as they were until their new analysis arrives; the symbol on show is asked
   * about again, and follows what its file's analysis finds. A revision already loaded is left alone
   * unless `force`.
   */
  private load(backend: Backend, revision: number, paths: readonly string[], force: boolean) {
    const had =
      this.backend && this.revision !== undefined ? { backend: this.backend, revision: this.revision } : undefined;
    const plan = planLoad(had, { backend, revision }, force);
    if (plan === "skip") return;
    this.reloadDue = false;
    this.backend = backend;
    this.revision = revision;
    this.paths = paths;
    this.loading?.abort();
    const { signal } = (this.loading = new AbortController());
    const wanted = new Set(paths);
    for (const path of [...this.files.keys()]) if (!wanted.has(path)) this.files.delete(path);
    for (const path of paths) if (!this.files.has(path)) this.files.set(path, { state: "loading" });
    this.progress = { done: 0, total: paths.length };
    if (plan === "new") this.cursor.reset();
    this.refresh();

    analyseFiles({
      paths,
      current: () => this.file,
      atOnce: FILES_AT_ONCE,
      signal,
      ask: (path, s) => backend.fileSymbols(revision, path, s),
      done: (path, outcome) => {
        let loaded: FileLoad;
        if (!outcome.ok) loaded = { state: "failed", message: errorText(outcome.error) };
        else if (outcome.value.gap) loaded = { state: "failed", message: outcome.value.gap };
        else loaded = { state: "done", symbols: outcome.value.symbols, context: outcome.value.context };
        this.files.set(path, loaded);
        this.progress = { done: this.progress.done + 1, total: this.progress.total };
        if (loaded.state === "done" && this.selected) {
          const now = reconcile(this.selected, { path, symbols: loaded.symbols });
          if (now !== this.selected) this.select(now);
        }
      },
    });
  }

  /** The symbol the review is to show was chosen: a row of the tree, or with `reveal` a symbol listed elsewhere. */
  choose(symbol: SymbolInfo, change?: SymbolChange, reveal = false) {
    this.cursor.reset();
    const shown = { symbol, change };
    this.select(shown);
    if (reveal) this.showInTree(shown);
  }

  /** The cursor of an editor is at a position of `side`'s text of `path` as of `revision`. */
  follow(revision: number, side: Side, path: string, line: number, col: number) {
    const place = { revision, side, path, line, col };
    if (reviewShown.on) this.cursor.move(place);
    else this.held = place;
  }

  /** Puts `shown` on show, unless it is. Answers whether it was not. */
  private select(shown: Shown): boolean {
    const now = this.selected;
    if (now && keyOf(now.symbol.id) === keyOf(shown.symbol.id) && now.change === shown.change) return false;
    this.selected = shown;
    this.unavailable = undefined;
    this.refresh();
    return true;
  }

  /** The symbol is where the cursor is: if it is a changed one, the tree shows its row. */
  private followed(shown: Shown) {
    if (this.select(shown)) this.showInTree(shown);
  }

  private showInTree(shown: Shown) {
    if (shown.change === undefined) return;
    // A removed symbol of a renamed file has the file's old path.
    const { path } = shown.symbol.id;
    const file = changeset.layout?.files.find((f) => f.path === path || f.oldPath === path);
    this.reveal = { symbol: shown.symbol, path: file?.path ?? path, n: ++this.revealed };
  }

  clearSelection() {
    this.leaveSelection();
    this.selected = this.signature = this.calls = this.usages = this.reach = this.removal = undefined;
    this.unavailable = undefined;
  }

  /** Asks again for what is shown about the selected symbol. */
  private refresh() {
    const { backend, revision, selected } = this;
    const run = this.leaveSelection();
    if (!selected || !backend || revision === undefined) return;
    const { signal } = (this.asking = new AbortController());
    const { id } = selected.symbol;
    this.signature = this.calls = this.usages = this.reach = this.removal = undefined;
    this.signature = this.start(
      run,
      () => backend.signature(revision, id, signal),
      (l) => {
        this.signature = l;
        // The symbol went with a change of the text.
        if (l.state === "failed" && isGone(l.message)) this.clearSelection();
      },
    );
    if (selected.change === "removed") {
      this.removal = this.start(
        run,
        () => backend.removalImpact(revision, id, signal),
        (l) => (this.removal = l),
      );
      return;
    }
    this.calls = this.start(
      run,
      () => backend.symbolCalls(revision, id, signal),
      (l) => (this.calls = l),
    );
    this.usages = this.start(
      run,
      () => backend.symbolUsages(revision, id, signal),
      (l) => (this.usages = l),
    );
    this.reach = this.start(
      run,
      () => backend.reachability(revision, id, signal),
      (l) => (this.reach = l),
    );
  }

  /** Cancels the requests about the selected symbol, in the backend too; returns the number of the next selection. */
  private leaveSelection(): number {
    this.asking?.abort();
    this.asking = undefined;
    return ++this.detailRun;
  }

  /** Runs `query`, hands the settled state to `set` unless another selection came first; returns the pending state. */
  private start<T>(run: number, query: () => Promise<T>, set: (load: Load<T>) => void): Load<T> {
    query().then(
      (value) => run === this.detailRun && set({ state: "done", value }),
      (error: unknown) =>
        run === this.detailRun && !isAbort(error) && set({ state: "failed", message: errorText(error) }),
    );
    return { state: "loading" };
  }
}

export const symbols = new SymbolsState();
