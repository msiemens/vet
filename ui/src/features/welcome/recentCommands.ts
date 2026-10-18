// The commands on the rows of the recent projects and the description of a row's context menu.

import type { MenuSpec } from "../../lib/commandMenu.ts";
import type { Command } from "../../lib/commands.ts";
import type { CommandContext } from "../../state/commands.ts";

export interface RecentActions {
  /** Opens the project at `path`, as a click on its row does. */
  open(path: string): void;
  /** Drops the project at `path` from the list. */
  forget(path: string): void;
  /** Removes the project at `path` with everything kept for it, after the viewer confirmed. */
  remove(path: string): void;
}

export function recentCommands(a: RecentActions): Command<CommandContext>[] {
  return [
    {
      id: "recent.open",
      label: "Open",
      subject: true,
      enabled: (c) => c.recentProject !== undefined,
      run: (c) => {
        if (!c.recentProject) return false;
        a.open(c.recentProject.path);
      },
    },
    {
      id: "recent.forget",
      label: "Remove from Recent Projects",
      subject: true,
      enabled: (c) => c.recentProject !== undefined,
      run: (c) => {
        if (!c.recentProject) return false;
        a.forget(c.recentProject.path);
      },
    },
    {
      id: "recent.remove",
      label: "Remove project…",
      subject: true,
      enabled: (c) => c.recentProject !== undefined,
      run: (c) => {
        if (!c.recentProject) return false;
        a.remove(c.recentProject.path);
      },
    },
  ];
}

/** The context menu of a recent project's row. */
export const recentMenu: MenuSpec = [
  { command: "recent.open" },
  "-",
  { command: "recent.forget" },
  { command: "recent.remove" },
];
