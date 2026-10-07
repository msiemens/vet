// The main menu, defined once. `buildMenu` returns the top-level menus; the host draws them
// as the native menu bar (macOS), the title bar's ☰ button shows them elsewhere. Both report
// a chosen item by its id, which `parseCommand` turns back into what to do.

import { KEYS } from "../lib/keymap.ts";
import { THEMES, type Theme } from "../lib/theme/theme.ts";
import { type MenuItem, separator } from "../lib/ui/menuItem.ts";
import { MODES, type Mode } from "../lib/viewMode.ts";

export interface MenuContext {
  /** The host draws the macOS menu bar, with its app menu and its own key accelerators. */
  native: boolean;
  /** A repository is open: its view and tool windows exist. */
  repoOpen: boolean;
  /** A review has started and can be finished. */
  reviewOpen: boolean;
  canQuit: boolean;
  /** Backend-side paths with their display names, most recent first. */
  recent: { path: string; name: string }[];
  mode: Mode;
  theme: Theme;
  tools: { id: string; label: string; shown: boolean }[];
  devPanel: boolean;
}

export type Command =
  | { type: "open" }
  | { type: "openRecent"; path: string }
  | { type: "finishReview" }
  | { type: "settings" }
  | { type: "about" }
  | { type: "quit" }
  | { type: "mode"; mode: Mode }
  | { type: "tool"; id: string }
  | { type: "theme"; theme: Theme }
  | { type: "devPanel" };

/** Also offered by the project chip's menu. */
export const openItem: MenuItem = { id: "open", label: "Open repository…" };

export function buildMenu(c: MenuContext): MenuItem[] {
  // The native menu bar's accelerator reports the item like a click; the window's own binding is `KEYS.settings`.
  const settings: MenuItem = {
    id: "settings",
    label: "Settings…",
    shortcut: c.native ? "Cmd+," : KEYS.settings.label,
  };
  const about: MenuItem = { id: "about", label: "About Vet" };
  const quit: MenuItem = {
    id: "quit",
    label: c.native ? "Quit Vet" : "Exit",
    enabled: c.canQuit,
    shortcut: c.native ? "Cmd+Q" : undefined,
  };

  const file: MenuItem[] = [
    openItem,
    {
      id: "recent",
      label: "Open Recent",
      enabled: c.recent.length > 0,
      children: c.recent.map(({ path, name }) => ({ id: `recent:${path}`, label: name, detail: path })),
    },
    separator,
    { id: "finishReview", label: "Finish Review…", enabled: c.reviewOpen },
    ...(c.native ? [] : [separator, settings, separator, quit]),
  ];
  const view: MenuItem[] = [
    ...MODES.map(({ id, label }) => ({
      id: `mode:${id}`,
      label,
      checked: c.mode === id,
      enabled: c.repoOpen,
    })),
    separator,
    {
      id: "tools",
      label: "Tool Windows",
      enabled: c.repoOpen,
      children: c.tools.map((t) => ({ id: `tool:${t.id}`, label: t.label, checked: t.shown })),
    },
    {
      id: "theme",
      label: "Theme",
      children: THEMES.map(({ id, label }) => ({ id: `theme:${id}`, label, checked: c.theme === id })),
    },
  ];
  const help: MenuItem[] = [
    ...(c.native ? [] : [about]),
    { id: "devPanel", label: "Developer Panel", checked: c.devPanel },
  ];

  return [
    ...(c.native ? [{ id: "app", label: "Vet", children: [about, separator, settings, separator, quit] }] : []),
    { id: "file", label: "File", children: file },
    { id: "view", label: "View", children: view },
    { id: "help", label: "Help", children: help },
  ];
}

export function parseCommand(id: string): Command | undefined {
  const [kind, ...rest] = id.split(":");
  const arg = rest.join(":");
  switch (kind) {
    case "open":
    case "settings":
    case "about":
    case "quit":
    case "finishReview":
      return { type: kind };
    case "devPanel":
      return { type: "devPanel" };
    case "recent":
      return { type: "openRecent", path: arg };
    case "mode": {
      const mode = MODES.find((m) => m.id === arg);
      return mode && { type: "mode", mode: mode.id };
    }
    case "tool":
      return { type: "tool", id: arg };
    case "theme": {
      const theme = THEMES.find((t) => t.id === arg);
      return theme && { type: "theme", theme: theme.id };
    }
    default:
      return undefined;
  }
}
