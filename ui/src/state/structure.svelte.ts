// The Structure tool's state: the outline of the file the active tab shows, the viewer's choices
// (sort and changed only, which are preferences) and the symbol selected in it. The backend derives the outline (SPEC §7.3, I5); this loads and holds it. The
// outline belongs to a tab: switching tabs starts it over, with every holder open.

import { untrack } from "svelte";
import { SvelteSet } from "svelte/reactivity";
import type { FileOutline } from "../generated/FileOutline";
import { isAbort } from "../lib/abort";
import { errorText } from "../lib/errorText";
import { toggleIn } from "../lib/foldKeys";
import { flagPreference } from "../lib/preferences.svelte";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import type { TabId } from "./commands";
import { whileReview } from "./reviewFollow";
import { reviewShown } from "./reviewShown.svelte";
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
  /** Where the caret of the tab's editor for the file is; none while no editor is mounted. */
  caret: () => Place | undefined;
}

interface Place {
  line: number;
  col: number;
}

const sortPreference = flagPreference("app", "structure.sort", false);
const changedOnlyPreference = flagPreference("app", "structure.changedOnly", false);

class StructureState {
  /** The symbols under each holder alphabetically. */
  get sort(): boolean {
    return sortPreference.value;
  }
  get changedOnly(): boolean {
    return changedOnlyPreference.value;
  }
  target = $state.raw<StructureTarget>();
  load = $state.raw<OutlineLoad>({ state: "loading" });
  /** The keys of the rows of holders that are shut. */
  closed = new SvelteSet<string>();
  /** The symbol selected, by `keyOf`. */
  selected = $state<string>();
  /** The popup over the editor is open. */
  popup = $state(false);

  private asking?: AbortController;
  /** The tab and file the outline held is of. */
  private loadedFor?: { tab: TabId; path: string };

  constructor() {
    // The outline belongs to the review screen; the store follows the project through it, and asks
    // nothing for a file nobody shows.
    whileReview((backend) => {
      const stopEffect = $effect.root(() => {
        $effect(() => {
          const target = this.target;
          untrack(() => this.fetch(backend, target));
        });
      });
      // A server installed or reconfigured may answer for files it had no answer for.
      const offEvents = backend.onEvent((event) => {
        if (event.type === "serversChanged" && event.project === backend.project) this.fetch(backend, this.target);
      });
      return () => {
        stopEffect();
        offEvents();
        this.asking?.abort();
        this.asking = undefined;
      };
    });
    session.onResync((backend) => {
      if (session.backend === backend && reviewShown.on) this.fetch(backend, this.target);
    });
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

  /**
   * Selects the symbol the caret of the tab's editor is in, once, and opens the holders around it;
   * with the caret in no symbol the selection is cleared.
   */
  revealCaret() {
    const { target, load } = this;
    const place = target?.caret();
    if (!place || load.state !== "done") return;
    const id = enclosingId(load.outline.symbols, place);
    this.selected = id;
    if (id !== undefined) for (const key of holdersOf(load.outline.symbols, id)) this.closed.delete(key);
  }

  /** The viewer chose a row: it is selected, and put on show in the Symbol panel where that works. */
  select(symbol: SymbolInfo, change?: SymbolChange) {
    this.selected = keyOf(symbol.id);
    if (this.target?.panel) symbols.choose(symbol, change);
  }

  toggleFold(key: string) {
    toggleIn(this.closed, key);
  }

  setSort(on: boolean) {
    sortPreference.value = on;
  }

  setChangedOnly(on: boolean) {
    changedOnlyPreference.value = on;
  }
}

export const structure = new StructureState();
