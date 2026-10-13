// What a click or a right click on a recent project's row does, wherever the rows are listed.

import { commands } from "../../state/commands";
import { contextMenus } from "../../state/contextMenu.svelte";
import { recentMenu } from "./recentCommands";

/** Opens the project at `path`. */
export const openRecent = (path: string) => commands.run(`file.openRecent:${path}`);

/** Opens the context menu of the row of the project at `path`, at the pointer. */
export function openRecentMenu(e: MouseEvent, path: string) {
  void contextMenus.open(recentMenu, e.clientX, e.clientY, { recentProject: { path } });
}
