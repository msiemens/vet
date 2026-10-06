// The main menu, defined once. `buildMenu` returns the top-level menus; the host draws them
// as the native menu bar (macOS), the title bar's ☰ button shows them elsewhere. Both report
// a chosen item by its id, which `parseCommand` turns back into what to do.

import type { Theme } from "../lib/theme/theme";
import { type MenuItem, separator } from "../lib/ui/menuItem.ts";

export interface MenuContext {
  mac: boolean;
  /** A repository is open: its view and tool windows exist. */
  repoOpen: boolean;
  canQuit: boolean;
  /** Backend-side paths with their display names, most recent first. */
  recent: { path: string; name: string }[];
  mode: "unified" | "split";
  theme: Theme;
  tools: { id: string; label: string; shown: boolean }[];
  devPanel: boolean;
}

export type Command =
  | { type: "open" }
  | { type: "openRecent"; path: string }
  | { type: "settings" }
  | { type: "about" }
  | { type: "quit" }
  | { type: "mode"; mode: MenuContext["mode"] }
  | { type: "tool"; id: string }
  | { type: "theme"; theme: Theme }
  | { type: "devPanel" };

const THEMES: [Theme, string][] = [
  ["system", "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];

export function buildMenu(c: MenuContext): MenuItem[] {
  const settings: MenuItem = { id: "settings", label: "Settings…", shortcut: c.mac ? "Cmd+," : "Ctrl+Alt+S" };
  const about: MenuItem = { id: "about", label: "About Vet" };
  const quit: MenuItem = {
    id: "quit",
    label: c.mac ? "Quit Vet" : "Exit",
    enabled: c.canQuit,
    shortcut: c.mac ? "Cmd+Q" : undefined,
  };

  const file: MenuItem[] = [
    { id: "open", label: "Open repository…" },
    {
      id: "recent",
      label: "Open Recent",
      enabled: c.recent.length > 0,
      children: c.recent.map(({ path, name }) => ({ id: `recent:${path}`, label: name, detail: path })),
    },
    ...(c.mac ? [] : [separator, settings, separator, quit]),
  ];
  const view: MenuItem[] = [
    ...(["unified", "split"] as const).map((mode) => ({
      id: `mode:${mode}`,
      label: mode === "unified" ? "Unified" : "Side by side",
      checked: c.mode === mode,
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
      children: THEMES.map(([theme, label]) => ({ id: `theme:${theme}`, label, checked: c.theme === theme })),
    },
  ];
  const help: MenuItem[] = [
    ...(c.mac ? [] : [about]),
    { id: "devPanel", label: "Developer Panel", checked: c.devPanel },
  ];

  return [
    ...(c.mac ? [{ id: "app", label: "Vet", children: [about, separator, settings, separator, quit] }] : []),
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
      return { type: kind };
    case "devPanel":
      return { type: "devPanel" };
    case "recent":
      return { type: "openRecent", path: arg };
    case "mode":
      return arg === "unified" || arg === "split" ? { type: "mode", mode: arg } : undefined;
    case "tool":
      return { type: "tool", id: arg };
    case "theme":
      return arg === "system" || arg === "light" || arg === "dark" ? { type: "theme", theme: arg } : undefined;
    default:
      return undefined;
  }
}
