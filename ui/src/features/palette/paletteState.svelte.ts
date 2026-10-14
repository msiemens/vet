// Search Everywhere's state: whether the popup is up, its tab and query, what was chosen lately in
// the open project, and the viewer's setting. The popup (`Palette.svelte`) draws it.

import { Preference, flagPreference } from "../../lib/preferences.svelte";
import { type CommandContext, commands } from "../../state/commands";
import { notices } from "../../state/notices.svelte";
import { Latest } from "../../state/resource";
import { session } from "../../state/session.svelte";
import { type PaletteTab, remember } from "./paletteRows";
import { type Found, SymbolSearch } from "./symbolSearch";

const unchangedPreference = flagPreference("app", "palette.unchanged", false);

/** The results chosen last in the open project, newest first. */
const recentPreference = new Preference<string[]>(
  "project",
  "palette.recent",
  (value) => (Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : undefined),
  [],
);

class PaletteState {
  open = $state(false);
  tab = $state<PaletteTab>("all");
  query = $state("");
  /** The ids of the results chosen last in the open project, newest first (`recentId`). */
  recent = $state.raw<string[]>([]);
  /**
   * The context the popup was opened in: the actions it lists say whether they can run there, since
   * the popup itself has the focus, which some commands read.
   */
  context = $state.raw<CommandContext>(commands.context());
  /** Files of the changeset's target that the changeset does not change are searched too. */
  get unchanged(): boolean {
    return unchangedPreference.value;
  }
  /** The paths of every file of the project, as of the snapshot `revision`; none until asked for. */
  files = $state.raw<{ revision: number; paths: string[] }>();
  /** The file list is being asked for. */
  listing = $state(false);
  /** Only the latest request for the file list counts. */
  private asked = new Latest();
  /** What the language servers found for a query on a revision; none until they answer. */
  served = $state.raw<Found>();
  private readonly search = new SymbolSearch(
    (query, revision, signal) => {
      const backend = session.backend;
      return backend ? backend.workspaceSymbols(revision, query, signal) : Promise.resolve([]);
    },
    {
      apply: (found) => (this.served = found),
      fail: (query, revision, error) => {
        this.served = { query, revision, languages: [] };
        notices.failed("Could not search the symbols", error);
      },
    },
  );

  constructor() {
    session.onClose(() => {
      this.asked.cancel();
      this.files = undefined;
      this.listing = false;
      this.forgetSymbols();
    });
  }

  /** Asks the language servers for the symbols named like `query`; the search under way is cancelled. */
  searchSymbols(query: string, revision: number) {
    this.search.search(query, revision);
  }

  /** Stops the search under way, keeping what it found before. */
  cancelSymbols() {
    this.search.cancel();
  }

  /** Drops what the servers found and stops asking them. */
  forgetSymbols() {
    this.search.cancel();
    this.served = undefined;
  }

  setUnchanged(on: boolean) {
    unchangedPreference.value = on;
  }

  /** Asks for the list of all files; an answer that an newer ask overtook is dropped. */
  async listFiles() {
    const backend = session.backend;
    if (!backend) return;
    const isLatest = this.asked.begin();
    this.listing = true;
    try {
      const listing = await backend.listFiles();
      if (isLatest()) this.files = listing;
    } catch (error) {
      if (isLatest()) notices.failed("Could not list the files", error);
    } finally {
      if (isLatest()) this.listing = false;
    }
  }

  /** Opens the popup at `tab`; one that is up moves to the tab and keeps the query. */
  show(tab: PaletteTab) {
    if (this.open) {
      this.tab = tab;
      return;
    }
    this.context = commands.context();
    this.recent = session.repo === undefined ? [] : recentPreference.value;
    this.query = "";
    this.tab = tab;
    this.open = true;
  }

  close() {
    this.open = false;
    this.forgetSymbols();
  }

  /** Remembers the result chosen, so it breaks ties from now on. */
  choose(id: string) {
    this.recent = remember(this.recent, id);
    if (session.repo !== undefined) recentPreference.value = this.recent;
  }
}

export const palette = new PaletteState();
