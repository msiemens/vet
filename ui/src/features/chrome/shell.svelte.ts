// The shell's own state: the screen shown and the tool windows of the rails and the dock. What `App`
// passes to the chrome and the review screen, and what the tool commands act on.

import { opened } from "../../state/opened.svelte";
import { reviewShown } from "../../state/reviewShown.svelte";
import { COMMITS, FALLBACK, lensOnSelect, toolFor } from "./commitsTool";
import { type Screen, TOOLS } from "./tools";
import { type ToolWindow, toggleBottom, toggleTool } from "./toggleTool";

class Shell {
  /** The queue is the project's home; the review screen is mounted only while it shows. */
  screen = $state<Screen>("queue");
  /** The left rail's chosen tool apart from the Commits one, and whether its sidebar is shown. */
  private leftChosen = $state<ToolWindow>({ tool: "changes", open: true });
  right = $state<ToolWindow>({ tool: "symbol", open: true });
  /** The bottom tool whose panel is shown; they exclude each other. */
  bottom = $state<string>();

  /** The left rail as it shows: the Commits tool while the changeset is reviewed by commit. */
  get left(): ToolWindow {
    const session = opened.session;
    return { tool: toolFor(this.leftChosen.tool, session?.lens, session?.commitCount ?? 0), open: this.leftChosen.open };
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
      this.bottom = toggleBottom(this.bottom, id);
      return this.bottom;
    }
    if (TOOLS.left.some((t) => t.id === id)) this.toggleLeft(id);
    else Object.assign(this.right, toggleTool(this.right, id));
    return undefined;
  }

  /** Shows the left rail's tool `tool`. */
  showLeft(tool: string) {
    this.selectLeft(tool);
  }

  /** Shows the bottom tool `tool`; the other bottom tools give way. */
  showBottom(tool: string) {
    this.bottom = tool;
  }

  /** Hides the bottom panel. */
  hideBottom() {
    this.bottom = undefined;
  }

  /** Shows or hides the left tool `id`: the Commits tool toggles its sidebar, any other selects it. */
  private toggleLeft(id: string) {
    if (id === COMMITS) {
      if (this.byCommit) this.leftChosen.open = !this.leftChosen.open;
      else this.selectLeft(id);
    } else if (this.byCommit) this.selectLeft(id);
    else Object.assign(this.leftChosen, toggleTool(this.leftChosen, id));
  }

  /**
   * Makes `tool` the left tool on show and asks for the lens it goes with. The Commits tool leaves
   * the chosen tool as it is, since the lens is what selects it, and gives way when there is nothing
   * to review by commit.
   */
  private selectLeft(tool: string) {
    if (tool === COMMITS && (opened.session?.commitCount ?? 0) <= 1) {
      this.leftChosen.tool = FALLBACK;
      this.leftChosen.open = true;
      return;
    }
    const lens = lensOnSelect(this.left.tool, tool);
    if (tool !== COMMITS) this.leftChosen.tool = tool;
    this.leftChosen.open = true;
    if (lens !== undefined) void opened.session?.showLens(lens);
  }

  /** Whether the changeset on view is reviewed by commit; the lens and the commit count say so. */
  private get byCommit(): boolean {
    return this.left.tool === COMMITS;
  }
}

export const shell = new Shell();
