// The review screen's commands and its context menus' descriptions. The commands name what they
// do; the controller and the screen supply how.

import type { Command } from "../../lib/commands.ts";
import type { MenuSpec } from "../../lib/commandMenu.ts";
import type { CommandContext, TabKind } from "../../state/commands.ts";

/** What the keyboard commands do in the active tab's view. */
export interface ViewActions {
  /** Moves the current hunk by `delta`. */
  step(delta: number): void;
  /** Flips the hunk at the focused editor's caret, else the current one; answers whether there was one. */
  toggleSeen(): boolean;
  /** Marks the hunk at the caret, else the current one, seen again if it is seen against an old base; answers whether there was one. */
  reconfirm(): boolean;
  /** Opens the comment form for the selection, else the current hunk; answers whether there was somewhere to comment. */
  comment(): boolean;
  /** Opens the file of the Changes stream's caret at its line; answers whether there was a caret. */
  jumpToSource(): boolean;
  /** Shows the commit `delta` away from the one on view; answers whether there was one. */
  stepCommit(delta: number): boolean;
}

export function viewCommands(a: ViewActions): Command<CommandContext>[] {
  const inView = (c: CommandContext) => c.diffView;
  return [
    {
      id: "review.nextHunk",
      label: "Next hunk",
      binding: "Ctrl-Alt-ArrowDown",
      inFields: true,
      enabled: inView,
      run: () => a.step(1),
    },
    {
      id: "review.previousHunk",
      label: "Previous hunk",
      binding: "Ctrl-Alt-ArrowUp",
      inFields: true,
      enabled: inView,
      run: () => a.step(-1),
    },
    {
      id: "review.previousCommit",
      label: "Previous commit",
      binding: "[",
      enabled: (c) => !!c.byCommit,
      run: () => a.stepCommit(-1),
    },
    {
      id: "review.nextCommit",
      label: "Next commit",
      binding: "]",
      enabled: (c) => !!c.byCommit,
      run: () => a.stepCommit(1),
    },
    { id: "review.toggleSeen", label: "Toggle seen", binding: "Space", enabled: inView, run: () => a.toggleSeen() },
    { id: "review.reconfirm", label: "Re-confirm", binding: "r", enabled: inView, run: () => a.reconfirm() },
    { id: "review.comment", label: "Add comment", binding: "Ctrl-Alt-c", enabled: inView, run: () => a.comment() },
    {
      id: "review.jumpToSource",
      label: "Jump to source",
      binding: "F4",
      inFields: true,
      enabled: (c) => c.diffView && c.tab?.kind === "changes",
      run: () => a.jumpToSource(),
    },
    {
      id: "review.goToDefinition",
      label: "Go to definition",
      binding: "Mod-b",
      enabled: (c) => c.editor !== undefined,
      run: (c) => c.editor?.goToDefinition(),
    },
  ];
}

/** What the commands of files and tabs do; the subject is the context's `target` and `tab`. */
export interface FileActions {
  copyText(text: string): void;
  excludePath(path: string, dir: boolean): void;
  reviewOnly(path: string): void;
  closeTab(kind: Exclude<TabKind, "changes">, path: string): void;
  /** Closes every tab but the Changes tab and this one, which gets the focus. */
  closeOtherTabs(kind: TabKind, path?: string): void;
}

export function fileCommands(a: FileActions): Command<CommandContext>[] {
  // A log tab has no path: what it shows is a check run.
  const pathOf = (c: CommandContext) => c.target?.path ?? (c.tab?.kind === "log" ? undefined : c.tab?.path);
  return [
    {
      id: "path.copy",
      label: "Copy path",
      enabled: (c) => pathOf(c) !== undefined,
      run: (c) => {
        const path = pathOf(c);
        if (path === undefined) return false;
        a.copyText(path);
      },
    },
    {
      id: "path.exclude",
      label: "Exclude from review",
      enabled: (c) => c.target !== undefined,
      run: (c) => {
        if (!c.target) return false;
        a.excludePath(c.target.path, c.target.dir);
      },
    },
    {
      id: "path.reviewOnly",
      label: "Review only this folder",
      enabled: (c) => !!c.target?.dir,
      run: (c) => {
        if (!c.target) return false;
        a.reviewOnly(c.target.path);
      },
    },
    {
      id: "tab.close",
      label: "Close tab",
      enabled: (c) => c.tab?.kind !== undefined && c.tab.kind !== "changes",
      run: (c) => {
        if (!c.tab || c.tab.kind === "changes" || c.tab.path === undefined) return false;
        a.closeTab(c.tab.kind, c.tab.path);
      },
    },
    {
      id: "tab.closeOthers",
      label: "Close other tabs",
      enabled: (c) => c.tab !== undefined,
      run: (c) => {
        if (!c.tab) return false;
        a.closeOtherTabs(c.tab.kind, c.tab.path);
      },
    },
    {
      id: "edit.copy",
      label: "Copy",
      enabled: (c) => !!c.editor?.selection,
      run: (c) => c.editor && a.copyText(c.editor.selectedText()),
    },
  ];
}

/** The context menu of a file or folder row in the file tree. */
export const treeMenu = (dir: boolean): MenuSpec => [
  { command: "path.copy" },
  { command: "path.exclude" },
  ...(dir ? [{ command: "path.reviewOnly" }] : []),
];

/** The context menu of a file's header in a view. */
export const fileHeaderMenu: MenuSpec = treeMenu(false);

/** The context menu of a diff or text editor. */
export const editorMenu: MenuSpec = [
  { command: "review.goToDefinition" },
  { command: "review.jumpToSource" },
  "-",
  { command: "review.comment" },
  { command: "review.toggleSeen" },
  { command: "review.reconfirm" },
  "-",
  { command: "edit.copy" },
];

/** The context menu of a tab. */
export const tabMenu: MenuSpec = [
  { command: "tab.close" },
  { command: "tab.closeOthers" },
  "-",
  { command: "path.copy" },
];
