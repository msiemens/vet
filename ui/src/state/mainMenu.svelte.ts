// The live main menu: the items for the current state, and the items chosen in it, from the
// title bar's ☰ menu or the host's native menu bar.

import { resolveMenu } from "../lib/commandMenu";
import type { MenuItem } from "../lib/ui/menuItem";
import { commands } from "./commands";
import { hasNativeMenu, onNativeMenu, setNativeMenu } from "./host";
import { type MenuState, mainMenuSpec } from "./mainMenu";

class MainMenu {
  /** The window draws the menu itself (☰ in the title bar); otherwise the host's menu bar does. */
  readonly native = hasNativeMenu;
  items = $state.raw<MenuItem[]>([]);
  /** The menu last sent to the native menu bar, so an unchanged menu is not rebuilt there. */
  private sent?: string;

  /**
   * Rebuilds the menu for `state` and the commands' context, and sends it to the native menu bar if
   * there is one. Runs inside an effect, so it must not read `items`: the effect would depend on
   * what it writes.
   */
  update(state: MenuState) {
    const items = resolveMenu(commands, mainMenuSpec(state), commands.context(), this.native);
    this.items = items;
    if (!this.native) return;
    const json = JSON.stringify(items);
    if (json === this.sent) return;
    this.sent = json;
    setNativeMenu(items).catch((e: unknown) => console.error(e));
  }

  /** Runs the items chosen in the native menu bar until the returned function is called. */
  listen(): () => void {
    const unlisten = this.native ? onNativeMenu((id) => commands.run(id)) : undefined;
    return () => void unlisten?.then((stop) => stop());
  }
}

export const mainMenu = new MainMenu();
