// The review screen's commands and its context menus' descriptions. The commands name what they
// do; the controller and the screen supply how.

import type { Command } from "../../lib/commands.ts";
import type { MenuSpec } from "../../lib/commandMenu.ts";
import { type ViewMode, modeCommand } from "../../lib/viewMode.ts";
import type { CommandContext, TabKind } from "../../state/commands.ts";

/** What the keyboard commands do in the active tab's view. */
export interface ViewActions {
  /** Moves the current hunk by `delta`. */
  step(delta: number): void;
  /** Flips the hunk at the focused editor's caret, else the current one; answers whether there was one. */
  toggleSeen(): boolean;
  /** Flips the flag of the hunk at the caret, else the current one; answers whether there was one. */
  toggleFlag(): boolean;
  /** Goes to the flagged hunk `delta` (1 or -1) from the current one, across files; answers whether there was one. */
  stepFlag(delta: number): boolean;
  /** Goes to the first flagged hunk of the changeset on view; answers whether there was one. */
  firstFlagged(): boolean;
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
  /** Shows the active tab in the view mode: a diff drawn that way, or the file's text. */
  setViewMode(mode: ViewMode): void;
}

/** The ways a tab shows its file, with the key that shows each. */
const VIEW_MODES: readonly { id: ViewMode; label: string; binding: string; unavailable: string }[] = [
  {
    id: "unified",
    label: "Unified Diff",
    binding: "Alt-Shift-1",
    unavailable: "Open the Changes tab or a changed file",
  },
  {
    id: "split",
    label: "Side-by-Side Diff",
    binding: "Alt-Shift-2",
    unavailable: "Open the Changes tab or a changed file",
  },
  { id: "file", label: "File Text", binding: "Alt-Shift-3", unavailable: "Open the tab of a file" },
];

const NO_DIFF = "Open the Changes tab or a changed file";
const NOT_BY_COMMIT = "The changeset is not reviewed by commit";
const QUEUE_SHOWN = "Open the Review screen first";

/** The commands, available only on the Review screen: the queue covers the review's views and tabs, whose keys it must not take. */
const onReviewScreen = (commands: Command<CommandContext>[]): Command<CommandContext>[] =>
  commands.map((command) => ({
    ...command,
    enabled: (c, arg) => !c.queueShown && (command.enabled?.(c, arg) ?? true),
    disabledReason: (c, arg) => (c.queueShown ? QUEUE_SHOWN : command.disabledReason?.(c, arg)),
  }));

export function viewCommands(a: ViewActions): Command<CommandContext>[] {
  const inView = (c: CommandContext) => c.diffView;
  const byCommit = (c: CommandContext) => !!c.byCommit;
  return onReviewScreen([
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
      id: "review.nextFlagged",
      label: "Next Flagged Hunk",
      binding: "Alt-f",
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.stepFlag(1),
    },
    {
      id: "review.previousFlagged",
      label: "Previous Flagged Hunk",
      binding: "Alt-Shift-f",
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.stepFlag(-1),
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
      yieldsToLists: true,
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.toggleSeen(),
    },
    {
      id: "review.toggleFlag",
      label: "Toggle Flag",
      binding: "f",
      enabled: inView,
      disabledReason: () => NO_DIFF,
      run: () => a.toggleFlag(),
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
      enabled: (c) => inView(c) && !c.noComments,
      disabledReason: (c) => (c.noComments ? "Comments are made in All changes" : NO_DIFF),
      run: () => a.comment(),
    },
    {
      id: "review.changeCard",
      label: "Show Change Card",
      binding: "Alt-F1",
      inFields: true,
      enabled: (c) => c.diffView && c.viewMode === "file",
      disabledReason: () => "Open the text of a changed file",
      run: () => a.showCard(),
    },
    {
      id: "review.jumpToSource",
      label: "Jump to Source",
      binding: "F4",
      inFields: true,
      enabled: (c) => c.diffView && c.viewMode !== "file",
      disabledReason: () => "Open the Changes tab or a changed file's diff",
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
      id: "nav.goToImplementation",
      label: "Go to Implementation",
      binding: "Mod-Alt-b",
      enabled: (c) => c.editor !== undefined && c.noImplementations === undefined,
      disabledReason: (c) =>
        c.noImplementations === undefined
          ? "Put the caret in an editor"
          : `${c.noImplementations} doesn't support Go to Implementation`,
      run: (c) => c.editor?.goToImplementation(),
    },
    {
      id: "nav.showUsages",
      label: "Show Usages",
      binding: "Mod-Alt-F7",
      enabled: (c) => c.editor !== undefined,
      disabledReason: () => "Put the caret in an editor",
      run: (c) => c.editor?.showUsages(),
    },
    ...VIEW_MODES.map(({ id, label, binding, unavailable }): Command<CommandContext> => ({
      id: modeCommand(id),
      label,
      binding,
      enabled: (c) => !!c.viewModes?.includes(id),
      disabledReason: (c) =>
        id !== "file" && c.tab?.kind === "file" ? "This file is unchanged: it has no diff" : unavailable,
      run: () => a.setViewMode(id),
    })),
  ]);
}

/** What the commands of files and tabs do; the subject is the context's `target` and `tab`. */
export interface FileActions {
  copyText(text: string): void;
  excludePath(path: string, dir: boolean): void;
  reviewOnly(path: string): void;
  /** Marks every changed file below the folder seen, or takes every mark back. */
  setFolderSeen(path: string, seen: boolean): void;
  closeTab(kind: Exclude<TabKind, "changes">, path: string, commit?: string): void;
  /** Closes every tab but the Changes tab and this one, which gets the focus. */
  closeOtherTabs(kind: TabKind, path?: string, commit?: string): void;
  /** Focuses the tab `delta` places from the active one, wrapping around. */
  stepTab(delta: number): void;
  /** Shows the file the review is at in the file tree; answers whether a tree is there to show it. */
  revealFile(): boolean;
}

export function fileCommands(a: FileActions): Command<CommandContext>[] {
  // A log tab has no path: what it shows is a check run.
  const pathOf = (c: CommandContext) => c.target?.path ?? (c.tab?.kind === "log" ? undefined : c.tab?.path);
  // The paths' commands also serve the queue's rows; tabs and the tree do not.
  const gated = (commands: Command<CommandContext>[]) => {
    const tabs = onReviewScreen(commands.filter((c) => /^(tab|view)\./.test(c.id)));
    return commands.map((c) => tabs.find((t) => t.id === c.id) ?? c);
  };
  return gated([
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
        a.closeTab(c.tab.kind, c.tab.path, c.tab.commit);
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
        a.closeOtherTabs(c.tab.kind, c.tab.path, c.tab.commit);
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
      binding: "Alt-F2",
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
  ]);
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
  { command: "nav.goToImplementation" },
  { command: "nav.showUsages" },
  { command: "review.changeCard" },
  { command: "review.jumpToSource" },
  "-",
  { command: "review.comment" },
  { command: "review.toggleSeen" },
  { command: "review.toggleFlag" },
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
