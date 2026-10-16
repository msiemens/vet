// The Symbol tool's state: the changed symbols of each changed file, which the Changes tree shows
// under their files, and the symbol on show, with the answers about it. The backend derives them
// (SPEC §7.3, I5) and analyses the files itself while the review screen shows the changeset, the
// files with an open tab first, pushing each file's answer as it comes (A13); this only applies
// what it hears. The selected symbol is the one the reviewer chose last: a row of the tree, or the
// symbol the cursor of an editor is in.

import { untrack } from "svelte";
import { SvelteMap } from "svelte/reactivity";
import type { Calls } from "../generated/Calls";
import type { ChangedSymbol } from "../generated/ChangedSymbol";
import type { ErrorKind } from "../generated/ErrorKind";
import type { FileSymbols } from "../generated/FileSymbols";
import type { Reachability } from "../generated/Reachability";
import type { RemovalImpact } from "../generated/RemovalImpact";
import type { Side } from "../generated/Side";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import type { SymbolSignature } from "../generated/SymbolSignature";
import type { Unavailable } from "../generated/Unavailable";
import type { Usages } from "../generated/Usages";
import { errorText } from "../lib/errorText";
import { isAbort } from "../lib/abort";
import { kindOf } from "../lib/requestError";
import type { Backend } from "./backend";
import { changeset } from "./changeset.svelte";
import { CursorFollow } from "./cursorFollow";
import { whileReview } from "./reviewFollow";
import { type IsLatest, Latest } from "./resource";
import { session } from "./session.svelte";
import { keyOf } from "./symbolKey";
import { reconcile, type Shown } from "./symbolLoad";

export type Load<T> =
  { state: "loading" } | { state: "done"; value: T } | { state: "failed"; message: string; kind?: ErrorKind };

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
  /** The files of the revision whose analysis ended, of those it is for. */
  progress = $state.raw({ done: 0, total: 0 });
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

  private backend?: Backend;
  private revision?: number;
  // The requests about the selected symbol; aborted when the selection moves.
  private asking?: AbortController;
  private details = new Latest();
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
    // The analysis the backend pushes is for the review screen; the store follows the project through
    // it, and holds nothing of a changeset nobody shows.
    whileReview((backend) => {
      const offEvents = backend.onEvent((event) => {
        if (event.project !== backend.project) return;
        if (event.type === "fileSymbols") this.apply(backend, event.symbols, event.done, event.total);
        // A server installed or reconfigured may answer again for the selected symbol.
        else if (event.type === "serversChanged") this.refresh();
      });
      const stopEffect = $effect.root(() => {
        $effect(() => {
          const layout = changeset.layout;
          // The semantic layer has the changeset's snapshots: not a commit's, and not a range's.
          const analysable = changeset.shownCommit === undefined && layout?.interdiff === undefined;
          const revision = layout?.shown.revision;
          const paths = layout?.files.filter((f) => f.hunks.length > 0).map((f) => f.path);
          untrack(() => {
            if (revision === undefined || !analysable || !paths) this.reset();
            else this.begin(backend, revision, paths);
            // The screen came back: the answers for the selected symbol are read again.
            if (this.selected !== undefined && this.signature === undefined) this.refresh();
          });
        });
      });
      return () => {
        offEvents();
        stopEffect();
        this.suspend();
      };
    });
    session.onClose(() => this.reset());
  }

  private requireBackend(): Backend {
    if (!this.backend) throw new Error("no repository is open");
    return this.backend;
  }

  /** Back to nothing: no repository is open, or no changeset to analyse is on view. */
  private reset() {
    this.backend = this.revision = undefined;
    this.files.clear();
    this.progress = { done: 0, total: 0 };
    this.cursor.reset();
    this.clearSelection();
  }

  /** The review screen went away: the analysis and the answers are dropped, the selected symbol stays. */
  private suspend() {
    this.asking?.abort();
    this.asking = undefined;
    this.details.begin();
    this.files.clear();
    this.progress = { done: 0, total: 0 };
    this.cursor.reset();
    this.signature = this.calls = this.usages = this.reach = this.removal = undefined;
    this.unavailable = undefined;
  }

  /**
   * The review screen shows the changeset at `revision`: the files wait for the backend's answers.
   * The files of the revision are marked loading so the tree's count moves; an answer replaces its
   * file's mark. A revision already followed is left alone: the backend pushes its answers once.
   */
  private begin(backend: Backend, revision: number, paths: readonly string[]) {
    if (this.backend !== backend) {
      // Another repository's changeset: the selected symbol is not this one's.
      this.backend = backend;
      this.revision = revision;
      this.cursor.reset();
      this.clearSelection();
    } else if (this.revision !== revision) {
      // The changeset moved: the selected symbol stays, and its answers are read for the new revision.
      this.revision = revision;
      this.signature = this.calls = this.usages = this.reach = this.removal = undefined;
      this.unavailable = undefined;
    } else if (this.files.size > 0) return;
    this.files.clear();
    for (const path of paths) this.files.set(path, { state: "loading" });
    this.progress = { done: 0, total: paths.length };
  }

  /** The backend analysed one file: apply what it found. */
  private apply(backend: Backend, symbols: FileSymbols, done: number, total: number) {
    if (backend !== this.backend || symbols.revision !== this.revision) return;
    const loaded: FileLoad = symbols.gap
      ? { state: "failed", message: symbols.gap.message }
      : { state: "done", symbols: symbols.symbols, context: symbols.context };
    this.files.set(symbols.path, loaded);
    this.progress = { done, total };
    if (loaded.state === "done" && this.selected) {
      const now = reconcile(this.selected, { path: symbols.path, symbols: loaded.symbols });
      if (now !== this.selected) this.select(now);
    }
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
    this.cursor.move({ revision, side, path, line, col });
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
        if (l.state === "failed" && l.kind === "notFound") this.clearSelection();
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

  /** Cancels the requests about the selected symbol, in the backend too; tells whether the next selection is still the latest. */
  private leaveSelection(): IsLatest {
    this.asking?.abort();
    this.asking = undefined;
    return this.details.begin();
  }

  /** Runs `query`, hands the settled state to `set` unless another selection came first; returns the pending state. */
  private start<T>(isLatest: IsLatest, query: () => Promise<T>, set: (load: Load<T>) => void): Load<T> {
    query().then(
      (value) => isLatest() && set({ state: "done", value }),
      (error: unknown) =>
        isLatest() && !isAbort(error) && set({ state: "failed", message: errorText(error), kind: kindOf(error) }),
    );
    return { state: "loading" };
  }
}

export const symbols = new SymbolsState();
