// The commands of the window as a whole: the ones the main menu offers. The owner of the state
// they change (`App`) supplies the actions.

import type { Command } from "../lib/commands.ts";
import { THEMES, type Theme } from "../lib/theme/theme.ts";
import { MODES, type Mode } from "../lib/viewMode.ts";
import type { CommandContext } from "./commands.ts";

export interface AppActions {
  pickProject(): void;
  openProject(path: string): void;
  finishReview(): void;
  showDialog(dialog: "settings" | "about"): void;
  quit(): void;
  setMode(mode: Mode): void;
  /** Shows or hides the tool window `id`. */
  toggleTool(id: string): void;
  setTheme(theme: Theme): void;
  toggleDevPanel(): void;
}

export function appCommands(a: AppActions): Command<CommandContext>[] {
  const repoOpen = (c: CommandContext) => c.repoOpen;
  return [
    { id: "file.open", label: "Open repository…", run: () => a.pickProject() },
    {
      id: "file.openRecent",
      label: "Open recent",
      run: (_, path) => {
        if (path === undefined) return false;
        a.openProject(path);
      },
    },
    { id: "review.finish", label: "Finish Review…", enabled: (c) => c.reviewOpen, run: () => a.finishReview() },
    {
      id: "app.settings",
      label: "Settings…",
      binding: "Ctrl-Alt-s",
      accelerator: "Cmd+,",
      inFields: true,
      run: () => a.showDialog("settings"),
    },
    { id: "app.about", label: "About Vet", run: () => a.showDialog("about") },
    { id: "app.quit", label: "Exit", accelerator: "Cmd+Q", enabled: (c) => c.canQuit, run: () => a.quit() },
    {
      id: "view.mode",
      label: "View mode",
      enabled: repoOpen,
      run: (_, id) => {
        const mode = MODES.find((m) => m.id === id);
        if (!mode) return false;
        a.setMode(mode.id);
      },
    },
    {
      id: "view.tool",
      label: "Tool window",
      enabled: repoOpen,
      run: (_, id) => {
        if (id === undefined) return false;
        a.toggleTool(id);
      },
    },
    {
      id: "view.theme",
      label: "Theme",
      run: (_, id) => {
        const theme = THEMES.find((t) => t.id === id);
        if (!theme) return false;
        a.setTheme(theme.id);
      },
    },
    { id: "dev.panel", label: "Developer Panel", run: () => a.toggleDevPanel() },
  ];
}
