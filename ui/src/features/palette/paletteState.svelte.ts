// Search Everywhere's state: whether the popup is up, its tab and query, what was chosen lately in
// the open project, and the viewer's two settings. The popup (`Palette.svelte`) draws it.

import type { LanguageSymbols } from "../../generated/LanguageSymbols";
import { isAbort } from "../../lib/abort";
import { errorText } from "../../lib/errorText";
import { readStored, writeStored } from "../../lib/storage";
import { type CommandContext, commands } from "../../state/commands";
import { session } from "../../state/session.svelte";
import { type PaletteTab, remember } from "./paletteRows";

const UNCHANGED_KEY = "vet.palette.unchanged";
const recentKey = (repo: string) => `vet.palette.recent:${repo}`;

/** The results chosen last in a project, newest first, as kept in the browser storage. */
function storedRecent(repo: string | undefined): string[] {
  if (repo === undefined) return [];
  try {
    const parsed: unknown = JSON.parse(readStored(recentKey(repo)) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}

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
  unchanged = $state(readStored(UNCHANGED_KEY) === "1");
  /** The paths of every file of the project, as of the snapshot `revision`; none until asked for. */
  files = $state.raw<{ revision: number; paths: string[] }>();
  /** The file list is being asked for. */
  listing = $state(false);
  /** Only the latest request for the file list counts. */
  private asked = 0;
  /** What the language servers found for `query`; none until they answer. */
  served = $state.raw<{ query: string; languages: LanguageSymbols[] }>();
  /** The servers are being asked. */
  serving = $state(false);
  private serverAsk?: AbortController;

  constructor() {
    session.onClose(() => {
      this.asked++;
      this.files = undefined;
      this.listing = false;
      this.forgetSymbols();
    });
  }

  /** Asks the language servers for the symbols named like `query`; the previous ask is cancelled. */
  async searchSymbols(query: string, revision: number) {
    const backend = session.backend;
    if (!backend) return;
    this.serverAsk?.abort();
    const ask = new AbortController();
    this.serverAsk = ask;
    this.serving = true;
    try {
      const languages = await backend.workspaceSymbols(revision, query, ask.signal);
      if (this.serverAsk === ask) this.served = { query, languages };
    } catch (error) {
      if (this.serverAsk === ask && !isAbort(error)) {
        this.served = { query, languages: [] };
        session.notice = `Could not search the symbols: ${errorText(error)}`;
      }
    } finally {
      if (this.serverAsk === ask) this.serving = false;
    }
  }

  /** Drops what the servers found and stops asking them. */
  forgetSymbols() {
    this.serverAsk?.abort();
    this.serverAsk = undefined;
    this.served = undefined;
    this.serving = false;
  }

  setUnchanged(on: boolean) {
    this.unchanged = on;
    writeStored(UNCHANGED_KEY, on ? "1" : undefined);
  }

  /** Asks for the list of all files; an answer that an newer ask overtook is dropped. */
  async listFiles() {
    const backend = session.backend;
    if (!backend) return;
    const ask = ++this.asked;
    this.listing = true;
    try {
      const listing = await backend.listFiles();
      if (ask === this.asked) this.files = listing;
    } catch (error) {
      if (ask === this.asked) session.notice = `Could not list the files: ${errorText(error)}`;
    } finally {
      if (ask === this.asked) this.listing = false;
    }
  }

  /** Opens the popup at `tab`; one that is up moves to the tab and keeps the query. */
  show(tab: PaletteTab) {
    if (this.open) {
      this.tab = tab;
      return;
    }
    this.context = commands.context();
    this.recent = storedRecent(session.repo);
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
    if (session.repo !== undefined) writeStored(recentKey(session.repo), JSON.stringify(this.recent));
  }
}

export const palette = new PaletteState();
