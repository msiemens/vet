// The window's context menus. Every one is a menu description opened at the pointer: the host
// pops it up natively, a plain browser has `App` draw `drawn`.

import type { MenuSpec } from "../lib/commandMenu";
import { type ContextMenuRequest, openContextMenu } from "../lib/contextMenu";
import { type CommandContext, commands, menuFor } from "./commands";
import { hasNativeContextMenu, popUpNativeMenu } from "./host";

class ContextMenus {
  /** The menu the page draws, if one is open. */
  drawn = $state.raw<ContextMenuRequest>();

  show(request: ContextMenuRequest): Promise<void> {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
    return openContextMenu(
      request,
      hasNativeContextMenu ? popUpNativeMenu : undefined,
      (r) => (this.drawn = r),
      () => previous?.focus({ preventScroll: true }),
    );
  }

  /** Opens the menu `spec` describes; `subject` is what it was opened on, and the choice runs in that context. */
  open(spec: MenuSpec, x: number, y: number, subject?: Partial<CommandContext>): Promise<void> {
    const { items, context } = menuFor(spec, subject);
    return this.show({ x, y, items, onselect: (id) => commands.run(id, context) });
  }

  close() {
    this.drawn = undefined;
  }
}

export const contextMenus = new ContextMenus();
