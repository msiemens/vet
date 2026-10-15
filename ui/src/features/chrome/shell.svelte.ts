// The shell's own state: the screen shown and the tool windows of the rails and the dock. What `App`
// passes to the chrome and the review screen, and what the tool commands act on.

import { type Screen, TOOLS } from "./tools";
import { type ToolWindow, toggleBottom, toggleTool } from "./toggleTool";

class Shell {
  /** The queue is the project's home; the review screen stays mounted behind it, so its views survive. */
  screen = $state<Screen>("queue");
  /** The selected tool of each rail, and whether its sidebar is shown. */
  left = $state<ToolWindow>({ tool: "changes", open: true });
  right = $state<ToolWindow>({ tool: "symbol", open: true });
  /** The bottom tool whose panel is shown; they exclude each other. */
  bottom = $state<string>();

  /** Shows the queue, or the review of the changeset on view. */
  show(screen: Screen) {
    this.screen = screen;
  }

  /** Shows or hides the tool `id`; answers the bottom tool whose panel is shown now, if `id` is one. */
  toggle(id: string): string | undefined {
    if (TOOLS.bottom.some((t) => t.id === id)) {
      this.bottom = toggleBottom(this.bottom, id);
      return this.bottom;
    }
    const rail = TOOLS.left.some((t) => t.id === id) ? this.left : this.right;
    Object.assign(rail, toggleTool(rail, id));
    return undefined;
  }

  /** Shows the left rail's tool `tool`. */
  showLeft(tool: string) {
    Object.assign(this.left, { tool, open: true });
  }

  /** Shows the bottom tool `tool`; the other bottom tools give way. */
  showBottom(tool: string) {
    this.bottom = tool;
  }

  /** Hides the bottom panel. */
  hideBottom() {
    this.bottom = undefined;
  }
}

export const shell = new Shell();
