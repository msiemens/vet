// The window's context menus. Every one is a menu description opened at the pointer: the host
// pops it up natively, a plain browser has `App` draw `drawn`.

import type { MenuSpec } from "../lib/commandMenu";
import { type ContextMenuRequest, openContextMenu } from "../lib/contextMenu";
import { focusScope } from "../lib/ui/focusScope";
import { type CommandContext, commands, menuFor } from "./commands";
import { hasNativeContextMenu, popUpNativeMenu } from "./host";

class ContextMenus {
  /** The menu the page draws, if one is open. */
  drawn = $state.raw<ContextMenuRequest>();

  /** `draw` shows the menu when the host has none; the window's by default. A dialog is modal, so its own menus are drawn inside it. */
  show(
    request: ContextMenuRequest,
    draw: (request: ContextMenuRequest) => void = (r) => (this.drawn = r),
  ): Promise<void> {
    // A menu the page draws keeps its own scope; this one is for the host's menu.
    const scope = focusScope();
    return openContextMenu(
      request,
      hasNativeContextMenu ? popUpNativeMenu : undefined,
      (r) => {
        scope.cancel();
        draw(r);
      },
      () => scope.leave(),
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
