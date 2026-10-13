// Search Everywhere's state: whether the popup is up, its tab and query, what was chosen lately in
// the open project, and the viewer's two settings. The popup (`Palette.svelte`) draws it.

import { readStored, writeStored } from "../../lib/storage";
import { type CommandContext, commands } from "../../state/commands";
import { session } from "../../state/session.svelte";
import { type PaletteTab, remember } from "./paletteRows";

const DOUBLE_SHIFT_KEY = "vet.palette.doubleShift";
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
  /** Two taps of Shift open the popup. */
  doubleShift = $state(readStored(DOUBLE_SHIFT_KEY) !== "0");
  /** The ids of the results chosen last in the open project, newest first (`recentId`). */
  recent = $state.raw<string[]>([]);
  /**
   * The context the popup was opened in: the actions it lists say whether they can run there, since
   * the popup itself has the focus, which some commands read.
   */
  context = $state.raw<CommandContext>(commands.context());

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
  }

  /** Remembers the result chosen, so it breaks ties from now on. */
  choose(id: string) {
    this.recent = remember(this.recent, id);
    if (session.repo !== undefined) writeStored(recentKey(session.repo), JSON.stringify(this.recent));
  }

  setDoubleShift(on: boolean) {
    this.doubleShift = on;
    writeStored(DOUBLE_SHIFT_KEY, on ? undefined : "0");
  }
}

export const palette = new PaletteState();
