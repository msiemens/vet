// The commands of the window as a whole: the ones the main menu offers. The owner of the state
// they change (`App`) supplies the actions.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { Command } from "../lib/commands.ts";
import { THEMES, type Theme } from "../lib/theme/theme.ts";
import { MODES, type Mode } from "../lib/viewMode.ts";
import { parseKeyId } from "./changesetKey.ts";
import type { CommandContext } from "./commands.ts";

export interface AppActions {
  pickProject(): void;
  openProject(path: string): void;
  finishReview(): void;
  /** Fetches the open project's remotes. */
  fetch(): void;
  /** Puts a changeset on view. */
  openChangeset(key: ChangesetKey): void;
  /** Shows the queue of changesets. */
  showQueue(): void;
  /** Opens the changeset switcher. */
  showChangesets(): void;
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
    { id: "file.open", label: "Open repository…", binding: "Mod-o", run: () => a.pickProject() },
    {
      id: "file.openRecent",
      label: "Open recent",
      argument: true,
      run: (_, path) => {
        if (path === undefined) return false;
        a.openProject(path);
      },
    },
    { id: "view.queue", label: "Open Queue", binding: "Mod-1", enabled: repoOpen, run: () => a.showQueue() },
    { id: "changeset.switch", label: "Switch Changeset…", enabled: repoOpen, run: () => a.showChangesets() },
    {
      id: "changeset.open",
      label: "Open changeset",
      enabled: repoOpen,
      run: (_, id) => {
        const key = parseKeyId(id);
        if (!key) return false;
        a.openChangeset(key);
      },
    },
    { id: "review.finish", label: "Finish Review…", enabled: (c) => c.reviewOpen, run: () => a.finishReview() },
    { id: "git.fetch", label: "Fetch", enabled: repoOpen, run: () => a.fetch() },
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
      argument: true,
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
      argument: true,
      enabled: repoOpen,
      run: (_, id) => {
        if (id === undefined) return false;
        a.toggleTool(id);
      },
    },
    {
      id: "view.theme",
      label: "Theme",
      argument: true,
      run: (_, id) => {
        const theme = THEMES.find((t) => t.id === id);
        if (!theme) return false;
        a.setTheme(theme.id);
      },
    },
    { id: "dev.panel", label: "Developer Panel", run: () => a.toggleDevPanel() },
  ];
}
