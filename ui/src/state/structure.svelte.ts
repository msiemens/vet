// The Structure tool's state: the outline of the file the active tab shows, the viewer's choices
// (sort, changed only and whether to follow the cursor, which are remembered) and the symbol
// selected in it. The backend derives the outline (SPEC §7.3, I5); this loads and holds it. The
// outline belongs to a tab: switching tabs starts it over, with every holder open.

import { untrack } from "svelte";
import { SvelteSet } from "svelte/reactivity";
import type { FileOutline } from "../generated/FileOutline";
import type { Side } from "../generated/Side";
import { isAbort } from "../lib/abort";
import { errorText } from "../lib/errorText";
import { readStored, writeStored } from "../lib/storage";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import type { TabId } from "./commands";
import { session } from "./session.svelte";
import type { Shown } from "./shown";
import { enclosingId, holdersOf } from "./structureRows";
import { keyOf } from "./symbolKey";
import { symbols } from "./symbols.svelte";
import { sameView } from "./view";

export type OutlineLoad =
  { state: "loading" } | { state: "done"; outline: FileOutline } | { state: "failed"; message: string };

/** The file whose structure is shown, and where it is read from. */
export interface StructureTarget {
  tab: TabId;
  path: string;
  /** The full id of the commit a tab stays at. */
  commit?: string;
  /** The revision and view the tab reads. */
  shown: Shown;
  /** The Symbol panel works for what the tab shows, so a symbol selected here is put on show there. */
  panel: boolean;
}

const PREFS = { sort: "vet.structure.sort", changedOnly: "vet.structure.changedOnly", follow: "vet.structure.follow" };

function flag(key: string, fallback: boolean): boolean {
  const stored = readStored(key);
  return stored === undefined ? fallback : stored === "1";
}

class StructureState {
  /** The symbols under each holder alphabetically. */
  sort = $state(flag(PREFS.sort, false));
  changedOnly = $state(flag(PREFS.changedOnly, false));
  /** The selection moves with the cursor of the editor. */
  follow = $state(flag(PREFS.follow, true));
  target = $state.raw<StructureTarget>();
  load = $state.raw<OutlineLoad>({ state: "loading" });
  /** The keys of the rows of holders that are shut. */
  closed = new SvelteSet<string>();
  /** The symbol selected, by `keyOf`. */
  selected = $state<string>();
  /** The popup over the editor is open. */
  popup = $state(false);
  /** The keyboard focus is in the window: the selection stays where the viewer put it. */
  focused = false;

  private asking?: AbortController;
  /** The tab and file the outline held is of. */
  private loadedFor?: { tab: TabId; path: string };

  constructor() {
    $effect.root(() => {
      $effect(() => {
        const { backend } = session;
        const target = this.target;
        untrack(() => this.fetch(backend, target));
      });
      // The symbol on show in the Symbol panel is where the cursor is, or what was last chosen.
      $effect(() => {
        const shown = symbols.selected;
        const on = this.follow && this.target?.panel;
        // eslint-disable-next-line @typescript-eslint/no-unused-expressions -- the read makes the outline a dependency
        this.load;
        if (on) untrack(() => this.moveTo(shown && keyOf(shown.symbol.id)));
      });
    });
    session.onResync((backend) => {
      if (session.backend === backend) this.reload();
    });
    // A server installed or reconfigured may answer for files it had no answer for.
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.type === "serversChanged" && event.project === backend.project) this.reload();
      }),
    );
  }

  /** The active tab shows the file `target`, or no file. */
  show(target: StructureTarget | undefined) {
    const now = this.target;
    const same =
      now === target ||
      (!!now &&
        !!target &&
        now.tab === target.tab &&
        now.path === target.path &&
        now.commit === target.commit &&
        now.panel === target.panel &&
        now.shown.revision === target.shown.revision &&
        sameView(now.shown.view, target.shown.view));
    if (!same) this.target = target;
  }

  private reload() {
    this.fetch(session.backend, this.target);
  }

  private fetch(backend: typeof session.backend, target: StructureTarget | undefined) {
    this.asking?.abort();
    if (!backend || !target) {
      this.loadedFor = undefined;
      this.load = { state: "loading" };
      return;
    }
    // The outline of the file shown stays until the new one arrives; another file's does not.
    if (this.loadedFor?.tab !== target.tab || this.loadedFor.path !== target.path) {
      this.loadedFor = { tab: target.tab, path: target.path };
      this.load = { state: "loading" };
      this.closed.clear();
      this.selected = undefined;
    }
    const { signal } = (this.asking = new AbortController());
    backend.fileOutline(target.shown.revision, target.shown.view, target.path, signal).then(
      (outline) => {
        if (!signal.aborted) this.load = { state: "done", outline };
      },
      (error: unknown) => {
        if (!signal.aborted && !isAbort(error)) this.load = { state: "failed", message: errorText(error) };
      },
    );
  }

  /** The cursor of the editor of `tab` is at a position of `side`'s text of `path`. */
  cursor(tab: TabId, side: Side, path: string, line: number, col: number) {
    const { target, load } = this;
    // Where the Symbol panel works it finds the symbol, for either side; elsewhere only the target's text is known.
    if (!this.follow || this.focused || !target || target.panel || load.state !== "done") return;
    if (target.tab !== tab && target.path !== path) return;
    if (side === "target") this.moveTo(enclosingId(load.outline.symbols, { line, col }));
  }

  /** Selects the symbol `id` and opens the holders around it; none clears the selection. */
  private moveTo(id: string | undefined) {
    if (this.focused) return;
    this.selected = id;
    if (id === undefined || this.load.state !== "done") return;
    for (const key of holdersOf(this.load.outline.symbols, id)) this.closed.delete(key);
  }

  /** The viewer chose a row: it is selected, and put on show in the Symbol panel where that works. */
  select(symbol: SymbolInfo, change?: SymbolChange) {
    this.selected = keyOf(symbol.id);
    if (this.target?.panel) symbols.choose(symbol, change);
  }

  toggleFold(key: string) {
    if (!this.closed.delete(key)) this.closed.add(key);
  }

  setSort(on: boolean) {
    this.sort = on;
    writeStored(PREFS.sort, on ? "1" : "0");
  }

  setChangedOnly(on: boolean) {
    this.changedOnly = on;
    writeStored(PREFS.changedOnly, on ? "1" : "0");
  }

  setFollow(on: boolean) {
    this.follow = on;
    writeStored(PREFS.follow, on ? "1" : "0");
  }
}

export const structure = new StructureState();
