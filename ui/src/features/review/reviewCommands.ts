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
  /** Opens the comment form for the last editor's selection or caret line, else the current hunk; answers whether there was somewhere to comment. */
  comment(): boolean;
  /** Opens the change card of the caret's line in a changed file's text; answers whether the line is a changed one. */
  showCard(): boolean;
  /** Opens the file of the Changes stream's caret at its line; answers whether there was a caret. */
  jumpToSource(): boolean;
  /** Shows the commit `delta` away from the one on view; answers whether there was one. */
  stepCommit(delta: number): boolean;
  /** Switches the active changed file between its diff and its text. */
  toggleFileText(): void;
}

const NO_DIFF = "Open the Changes tab or a changed file";
const NOT_BY_COMMIT = "The changeset is not reviewed by commit";

export function viewCommands(a: ViewActions): Command<CommandContext>[] {
  const inView = (c: CommandContext) => c.diffView;
  const byCommit = (c: CommandContext) => !!c.byCommit;
  return [
    {
      id: "review.nextHunk",
      label: "Next Hunk",
      binding: "Ctrl-Alt-ArrowDown",
      inFields: true,
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.step(1),
    },
    {
      id: "review.previousHunk",
      label: "Previous Hunk",
      binding: "Ctrl-Alt-ArrowUp",
      inFields: true,
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.step(-1),
    },
    {
      id: "review.previousCommit",
      label: "Previous Commit",
      binding: "[",
      enabled: byCommit,
      disabledReason: () => NOT_BY_COMMIT,
      run: () => a.stepCommit(-1),
    },
    {
      id: "review.nextCommit",
      label: "Next Commit",
      binding: "]",
      enabled: byCommit,
      disabledReason: () => NOT_BY_COMMIT,
      run: () => a.stepCommit(1),
    },
    {
      id: "review.toggleSeen",
      label: "Toggle Seen",
      binding: "Space",
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.toggleSeen(),
    },
    {
      id: "review.reconfirm",
      label: "Re-confirm",
      binding: "r",
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.reconfirm(),
    },
    {
      id: "review.comment",
      label: "Add Comment",
      binding: "Ctrl-Alt-c",
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.comment(),
    },
    {
      id: "review.changeCard",
      label: "Show Change Card",
      binding: "Alt-F1",
      inFields: true,
      enabled: (c) => c.diffView && c.tab?.kind === "text",
      disabledReason: () => "Open the text of a changed file",
      run: () => a.showCard(),
    },
    {
      id: "review.jumpToSource",
      label: "Jump to Source",
      binding: "F4",
      inFields: true,
      enabled: (c) => c.diffView && c.tab?.kind === "changes",
      disabledReason: () => "Open the Changes tab",
      run: () => a.jumpToSource(),
    },
    {
      id: "review.goToDefinition",
      label: "Go to Definition",
      binding: "Mod-b",
      enabled: (c) => c.editor !== undefined,
      disabledReason: () => "Put the caret in an editor",
      run: (c) => c.editor?.goToDefinition(),
    },
    {
      id: "view.fileText",
      label: "Show File Text",
      enabled: (c) => c.fileView !== undefined,
      disabledReason: () => "Open the tab of a changed file",
      run: () => a.toggleFileText(),
    },
  ];
}

/** What the commands of files and tabs do; the subject is the context's `target` and `tab`. */
export interface FileActions {
  copyText(text: string): void;
  excludePath(path: string, dir: boolean): void;
  reviewOnly(path: string): void;
  /** Marks every changed file below the folder seen, or takes every mark back. */
  setFolderSeen(path: string, seen: boolean): void;
  closeTab(kind: Exclude<TabKind, "changes">, path: string): void;
  /** Closes every tab but the Changes tab and this one, which gets the focus. */
  closeOtherTabs(kind: TabKind, path?: string): void;
  /** Focuses the tab `delta` places from the active one, wrapping around. */
  stepTab(delta: number): void;
  /** Shows the file the review is at in the file tree; answers whether a tree is there to show it. */
  revealFile(): boolean;
}

export function fileCommands(a: FileActions): Command<CommandContext>[] {
  // A log tab has no path: what it shows is a check run.
  const pathOf = (c: CommandContext) => c.target?.path ?? (c.tab?.kind === "log" ? undefined : c.tab?.path);
  return [
    {
      id: "path.copy",
      label: "Copy Path",
      enabled: (c) => pathOf(c) !== undefined,
      run: (c) => {
        const path = pathOf(c);
        if (path === undefined) return false;
        a.copyText(path);
      },
    },
    {
      id: "path.exclude",
      label: "Exclude from Review",
      enabled: (c) => c.target !== undefined,
      run: (c) => {
        if (!c.target) return false;
        a.excludePath(c.target.path, c.target.dir);
      },
    },
    {
      id: "path.reviewOnly",
      label: "Review Only This Folder",
      enabled: (c) => !!c.target?.dir,
      run: (c) => {
        if (!c.target) return false;
        a.reviewOnly(c.target.path);
      },
    },
    {
      id: "path.markSeen",
      label: "Mark All Seen",
      enabled: (c) => !!c.target?.dir,
      run: (c) => {
        if (!c.target) return false;
        a.setFolderSeen(c.target.path, true);
      },
    },
    {
      id: "path.markUnseen",
      label: "Mark All Unseen",
      enabled: (c) => !!c.target?.dir,
      run: (c) => {
        if (!c.target) return false;
        a.setFolderSeen(c.target.path, false);
      },
    },
    {
      id: "tab.close",
      label: "Close Tab",
      binding: "Mod-w",
      accelerator: "Cmd+W",
      enabled: (c) => c.tab?.kind !== undefined && c.tab.kind !== "changes",
      disabledReason: (c) => (c.tab ? "The Changes tab cannot be closed" : "No tab is open"),
      run: (c) => {
        if (!c.tab || c.tab.kind === "changes" || c.tab.path === undefined) return false;
        a.closeTab(c.tab.kind, c.tab.path);
      },
    },
    {
      id: "tab.closeOthers",
      label: "Close Other Tabs",
      // What stays is the Changes tab and the tab itself.
      enabled: (c) => c.tab !== undefined && (c.tabCount ?? 0) > (c.tab.kind === "changes" ? 1 : 2),
      disabledReason: () => "No other tab can be closed",
      run: (c) => {
        if (!c.tab) return false;
        a.closeOtherTabs(c.tab.kind, c.tab.path);
      },
    },
    {
      id: "tab.previous",
      label: "Previous Tab",
      binding: "Alt-ArrowLeft",
      macBinding: "Meta-Shift-BracketLeft",
      enabled: (c) => (c.tabCount ?? 0) > 1,
      disabledReason: () => "Only one tab is open",
      run: () => a.stepTab(-1),
    },
    {
      id: "tab.next",
      label: "Next Tab",
      binding: "Alt-ArrowRight",
      macBinding: "Meta-Shift-BracketRight",
      enabled: (c) => (c.tabCount ?? 0) > 1,
      disabledReason: () => "Only one tab is open",
      run: () => a.stepTab(1),
    },
    {
      id: "view.revealFile",
      label: "Reveal File in Tree",
      enabled: (c) => !!c.changesetOpen,
      disabledReason: () => "No changeset is on view",
      run: () => a.revealFile(),
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
  ...(dir
    ? [{ command: "path.reviewOnly" }, "-" as const, { command: "path.markSeen" }, { command: "path.markUnseen" }]
    : []),
];

/** The context menu of a file's header in a view. */
export const fileHeaderMenu: MenuSpec = treeMenu(false);

/** The context menu of a diff or text editor. */
export const editorMenu: MenuSpec = [
  { command: "review.goToDefinition" },
  { command: "review.changeCard" },
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
