// The Structure tool's state: the outline of the file the active tab shows, and the symbol selected in
// it, which belong to one opened changeset (`SessionStructure`); and the viewer's choices (sort and
// changed only, which are preferences) and the popup, which are the window's (`structure`). The backend
// derives the outline (SPEC §6.3); this loads and holds it. The outline belongs to a tab: switching
// tabs starts it over, with every holder open.

import { untrack } from "svelte";
import { SvelteSet } from "svelte/reactivity";
import type { FileOutline } from "../generated/FileOutline";
import { isAbort } from "../lib/abort";
import { errorText } from "../lib/errorText";
import { toggleIn } from "../lib/foldKeys";
import { flagPreference } from "../lib/preferences.svelte";
import type { SymbolChange } from "../generated/SymbolChange";
import type { SymbolInfo } from "../generated/SymbolInfo";
import type { ChangesetSession } from "./changesetSession.svelte";
import type { TabId } from "./commands";
import type { SessionEvent } from "./projectEvents";
import { session } from "./session.svelte";
import type { Shown } from "./shown";
import { enclosingId, holdersOf } from "./structureRows";
import { keyOf } from "./symbolKey";
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

/** The window's choices about how the Structure tool lists symbols. */
class StructureState {
  /** The symbols under each holder alphabetically. */
  get sort(): boolean {
    return sortPreference.value;
  }
  get changedOnly(): boolean {
    return changedOnlyPreference.value;
  }
  /** The popup over the editor is open. */
  popup = $state(false);

  setSort(on: boolean) {
    sortPreference.value = on;
  }

  setChangedOnly(on: boolean) {
    changedOnlyPreference.value = on;
  }
}

export const structure = new StructureState();

class SessionStructure {
  target = $state.raw<StructureTarget>();
  load = $state.raw<OutlineLoad>({ state: "loading" });
  /** The keys of the rows of holders that are shut. */
  closed = new SvelteSet<string>();
  /** The symbol selected, by `keyOf`. */
  selected = $state<string>();

  private asking?: AbortController;
  /** The tab and file the outline held is of. */
  private loadedFor?: { tab: TabId; path: string };
  private following = false;

  constructor(private owner: ChangesetSession) {}

  /**
   * The review screen shows the changeset: the outline of the target is read, and again when a server
   * is installed or reconfigured, which may answer for files it had no answer for. Returns what lets go
   * of it when the screen goes away.
   */
  followReview(): () => void {
    this.following = true;
    const stopEffect = $effect.root(() => {
      $effect(() => {
        const target = this.target;
        untrack(() => this.fetch(target));
      });
    });
    return () => {
      this.following = false;
      stopEffect();
      this.asking?.abort();
      this.asking = undefined;
    };
  }

  /** An event of the project, while the review screen is shown. */
  hear(event: SessionEvent) {
    if (this.following && event.type === "serversChanged") this.fetch(this.target);
  }

  /** The connection came back: the outline is read again. */
  resync() {
    if (this.following) this.fetch(this.target);
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

  private fetch(target: StructureTarget | undefined) {
    this.asking?.abort();
    if (!target) {
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
    session
      .connected()
      .fileOutline(target.shown.revision, target.shown.view, target.path, signal)
      .then(
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
    if (this.target?.panel) this.owner.symbols.choose(symbol, change);
  }

  toggleFold(key: string) {
    toggleIn(this.closed, key);
  }
}

export type { SessionStructure };

export const newStructure = (owner: ChangesetSession): SessionStructure => new SessionStructure(owner);
