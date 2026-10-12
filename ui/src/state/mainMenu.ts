// The main menu, defined once as a description of which commands it offers (see `lib/commandMenu`).
// Labels, shortcuts and enabled state come from the command registry. The host draws the result
// as the native menu bar (macOS), the title bar's ☰ button shows it elsewhere. Both report a chosen
// item by its id, which `commands.run` executes.

import type { MenuSpec } from "../lib/commandMenu.ts";
import { THEMES, type Theme } from "../lib/theme/theme.ts";
import { MODES, type Mode } from "../lib/viewMode.ts";
import { toolCommand } from "./appCommands.ts";

/** What the menu shows that is not a command's own: the lists it offers and what is checked in them. */
export interface MenuState {
  /** The host draws the macOS menu bar, with its app menu and its own key accelerators. */
  native: boolean;
  /** Backend-side paths with their display names, most recent first. */
  recent: { path: string; name: string }[];
  mode: Mode;
  theme: Theme;
  /** The tool windows by id, and which are shown. */
  tools: { id: string; shown: boolean }[];
}

/** Also offered by the project chip's menu. */
export const openItem: MenuSpec = [{ command: "file.open" }];

export function mainMenuSpec(s: MenuState): MenuSpec {
  const settings = { command: "app.settings" };
  const about = { command: "app.about" };
  const quit = { command: "app.quit", label: s.native ? "Quit Vet" : undefined };

  const file: MenuSpec = [
    ...openItem,
    {
      id: "recent",
      label: "Open Recent",
      items: s.recent.map(({ path, name }) => ({ command: "file.openRecent", arg: path, label: name, detail: path })),
    },
    "-",
    { command: "changeset.switch" },
    { command: "changeset.openPullRequest" },
    { command: "review.sinceLast" },
    { command: "review.finish" },
    "-",
    { command: "semantic.clearCaches" },
    ...(s.native ? [] : (["-", settings, "-", quit] as MenuSpec)),
  ];
  const view: MenuSpec = [
    { command: "view.queue" },
    "-",
    ...MODES.map(({ id, label }) => ({ command: "view.mode", arg: id, label, checked: s.mode === id })),
    "-",
    {
      id: "tools",
      label: "Tool Windows",
      items: s.tools.map((t) => ({ command: toolCommand(t.id), checked: t.shown })),
    },
    {
      id: "theme",
      label: "Theme",
      items: THEMES.map(({ id, label }) => ({ command: "view.theme", arg: id, label, checked: s.theme === id })),
    },
  ];
  const vcs: MenuSpec = [{ command: "git.fetch" }];
  const help: MenuSpec = [...(s.native ? [] : [about]), { command: "help.showLog" }];

  return [
    ...(s.native ? [{ id: "app", label: "Vet", items: [about, "-", settings, "-", quit] as MenuSpec }] : []),
    { id: "file", label: "File", items: file },
    { id: "view", label: "View", items: view },
    { id: "vcs", label: "VCS", items: vcs },
    { id: "help", label: "Help", items: help },
  ];
}
