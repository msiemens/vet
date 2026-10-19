// The shell's own state: the screen shown and the tool windows of the rails and the dock. What `App`
// passes to the chrome and the review screen, and what the tool commands act on.

import { Preference } from "../../lib/preferences.svelte";
import { opened } from "../../state/opened.svelte";
import { reviewShown } from "../../state/reviewShown.svelte";
import { COMMITS, FALLBACK, lensOnSelect, toolFor } from "./commitsTool";
import { type Screen, TOOLS } from "./tools";
import { type ToolWindow, parseToolWindow, toggleBottom, toggleTool } from "./toggleTool";

const toolIds = (tools: readonly { id: string }[]) => tools.map((t) => t.id);

// The tool windows come back as the viewer left them; one that is gone falls back to the default.
const leftPreference = new Preference<ToolWindow>(
  "app",
  "shell.left",
  (json) =>
    parseToolWindow(
      json,
      toolIds(TOOLS.left).filter((id) => id !== COMMITS),
    ),
  { tool: FALLBACK, open: true },
);
const rightPreference = new Preference<ToolWindow>(
  "app",
  "shell.right",
  (json) => parseToolWindow(json, toolIds(TOOLS.right)),
  { tool: "symbol", open: true },
);
const bottomPreference = new Preference<string | undefined>(
  "app",
  "shell.bottom",
  (json) => toolIds(TOOLS.bottom).find((id) => id === json),
  undefined,
);

class Shell {
  /** The queue is the project's home; the review screen is mounted only while it shows. */
  screen = $state<Screen>("queue");

  /** The left rail's chosen tool apart from the Commits one, and whether its sidebar is shown. */
  private get leftChosen(): ToolWindow {
    return leftPreference.value;
  }

  get right(): ToolWindow {
    return rightPreference.value;
  }

  /** The bottom tool whose panel is shown; they exclude each other. */
  get bottom(): string | undefined {
    return bottomPreference.value;
  }

  /** Calls `listener` whenever the bottom tool shown changes, also when the kept choice is loaded. */
  watchBottom(listener: () => void): () => void {
    return bottomPreference.watch(listener);
  }

  /** The left rail as it shows: the Commits tool while the changeset is reviewed by commit. */
  get left(): ToolWindow {
    const session = opened.session;
    return {
      tool: toolFor(this.leftChosen.tool, session?.lens, session?.commitCount ?? 0),
      open: this.leftChosen.open,
    };
  }

  /** Shows the queue, or the review of the changeset on view. */
  show(screen: Screen) {
    this.screen = screen;
    // The servers of a changeset are for the review screen, so the backend is told at the source.
    reviewShown.set(screen === "review");
  }

  /** Shows or hides the tool `id`; answers the bottom tool whose panel is shown now, if `id` is one. */
  toggle(id: string): string | undefined {
    if (TOOLS.bottom.some((t) => t.id === id)) {
      bottomPreference.value = toggleBottom(this.bottom, id);
      return this.bottom;
    }
    if (TOOLS.left.some((t) => t.id === id)) this.toggleLeft(id);
    else rightPreference.value = toggleTool(this.right, id);
    return undefined;
  }

  /** Shows the left rail's tool `tool`. */
  showLeft(tool: string) {
    this.selectLeft(tool);
  }

  /** Shows the bottom tool `tool`; the other bottom tools give way. */
  showBottom(tool: string) {
    bottomPreference.value = tool;
  }

  /** Hides the left sidebar; its tool stays selected, as when the rail hides it. */
  hideLeft() {
    leftPreference.value = { ...this.leftChosen, open: false };
  }

  /** Hides the right sidebar; its tool stays selected, as when the rail hides it. */
  hideRight() {
    rightPreference.value = { ...this.right, open: false };
  }

  /** Hides the bottom panel. */
  hideBottom() {
    bottomPreference.value = undefined;
  }

  /** Shows or hides the left tool `id`: the Commits tool toggles its sidebar, any other selects it. */
  private toggleLeft(id: string) {
    if (id === COMMITS) {
      if (this.byCommit) leftPreference.value = { ...this.leftChosen, open: !this.leftChosen.open };
      else this.selectLeft(id);
    } else if (this.byCommit) this.selectLeft(id);
    else leftPreference.value = toggleTool(this.leftChosen, id);
  }

  /**
   * Makes `tool` the left tool on show and asks for the lens it goes with. The Commits tool leaves
   * the chosen tool as it is, since the lens is what selects it, and gives way when there is nothing
   * to review by commit.
   */
  private selectLeft(tool: string) {
    if (tool === COMMITS && (opened.session?.commitCount ?? 0) <= 1) {
      leftPreference.value = { tool: FALLBACK, open: true };
      return;
    }
    const lens = lensOnSelect(this.left.tool, tool);
    leftPreference.value = { tool: tool === COMMITS ? this.leftChosen.tool : tool, open: true };
    if (lens !== undefined) void opened.session?.showLens(lens);
  }

  /** Whether the changeset on view is reviewed by commit; the lens and the commit count say so. */
  private get byCommit(): boolean {
    return this.left.tool === COMMITS;
  }
}

export const shell = new Shell();
