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
  /** Shows what changed since the version last reviewed. */
  changesSinceReview(): void;
  /** Fetches the open project's remotes. */
  fetch(): void;
  /** Puts a changeset on view. */
  openChangeset(key: ChangesetKey): void;
  /** Shows the queue of changesets. */
  showQueue(): void;
  /** Opens the changeset switcher. */
  showChangesets(): void;
  /** Asks for a pull request by its number. */
  openPullRequest(): void;
  showDialog(dialog: "settings" | "about"): void;
  quit(): void;
  setMode(mode: Mode): void;
  /** Shows or hides the tool window `id`. */
  toggleTool(id: string): void;
  setTheme(theme: Theme): void;
  /** Clears the backend's semantic caches. */
  clearSemanticCaches(): void;
  /** Opens the folder of the log files, or says where it is. */
  showLog(): void;
}

/** The tool windows of the two rails, with the key that shows or hides each. */
export const TOOL_WINDOWS: readonly { side: "left" | "right"; id: string; label: string; binding: string }[] = [
  { side: "left", id: "changes", label: "Changes", binding: "Ctrl-Alt-1" },
  { side: "left", id: "all", label: "All files", binding: "Ctrl-Alt-2" },
  { side: "left", id: "commits", label: "Commits", binding: "Ctrl-Alt-3" },
  { side: "left", id: "comments", label: "Comments", binding: "Ctrl-Alt-4" },
  { side: "right", id: "symbol", label: "Symbol", binding: "Ctrl-Alt-5" },
];

/** The id of the command that shows or hides the tool window `id`. */
export const toolCommand = (id: string) => `view.tool.${id}`;

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
      id: "changeset.openPullRequest",
      label: "Open Pull Request…",
      binding: "Mod-Shift-o",
      enabled: repoOpen,
      run: () => a.openPullRequest(),
    },
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
    {
      id: "review.sinceLast",
      label: "Changes Since My Last Review",
      enabled: (c) => c.reviewOpen,
      run: () => a.changesSinceReview(),
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
    ...TOOL_WINDOWS.map(({ id, label, binding }): Command<CommandContext> => ({
      id: toolCommand(id),
      label,
      binding,
      enabled: id === "commits" ? (c) => c.repoOpen && !!c.manyCommits : repoOpen,
      run: () => a.toggleTool(id),
    })),
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
    { id: "semantic.clearCaches", label: "Clear Semantic Caches", run: () => a.clearSemanticCaches() },
    { id: "help.showLog", label: "Show Log", run: () => a.showLog() },
  ];
}
