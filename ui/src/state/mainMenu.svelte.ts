// The live main menu: the items for the current state, and the commands chosen in it, from
// the title bar's ☰ menu or the host's native menu bar.

import type { MenuItem } from "../lib/ui/menuItem";
import { hasNativeMenu, onNativeMenu, setNativeMenu } from "./host";
import { type Command, type MenuContext, buildMenu, parseCommand } from "./mainMenu";

class MainMenu {
  /** The window draws the menu itself (☰ in the title bar); otherwise the host's menu bar does. */
  readonly native = hasNativeMenu;
  items = $state.raw<MenuItem[]>([]);
  private handler?: (command: Command) => void;
  /** The menu last sent to the native menu bar, so an unchanged menu is not rebuilt there. */
  private sent?: string;

  /**
   * Rebuilds the menu for `context`, and sends it to the native menu bar if there is one.
   * Runs inside an effect, so it must not read `items`: the effect would depend on what it writes.
   */
  update(context: MenuContext) {
    const items = buildMenu(context);
    this.items = items;
    if (!this.native) return;
    const json = JSON.stringify(items);
    if (json === this.sent) return;
    this.sent = json;
    setNativeMenu(items).catch((e: unknown) => console.error(e));
  }

  /** `handler` executes the chosen commands until the returned function is called. */
  listen(handler: (command: Command) => void): () => void {
    this.handler = handler;
    const unlisten = this.native ? onNativeMenu((id) => this.run(id)) : undefined;
    return () => {
      this.handler = undefined;
      void unlisten?.then((stop) => stop());
    };
  }

  /** An item was chosen. */
  run(id: string) {
    const command = parseCommand(id);
    if (command) this.handler?.(command);
  }
}

export const mainMenu = new MainMenu();
