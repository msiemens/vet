// The window's one command registry, and the facts commands read to decide whether they apply.
// Features register their commands with it and contribute the facts they own; the key handler,
// the menus and the tooltips read it.

import type { Side } from "../generated/Side";
import type { Thread } from "../generated/Thread";
import { CommandRegistry } from "../lib/commands";
import { type MenuSpec, resolveMenu } from "../lib/commandMenu";
import { bindingSignal } from "./bindingSignal.svelte";
import { isMac } from "./host";

export type TabKind = "changes" | "file" | "text" | "log";

/** The editor that has the focus. */
export interface FocusedEditor {
  /** Which text it shows: the target, or the base of a side-by-side view. */
  side: Side;
  path: string;
  /** Some text is selected. */
  selection: boolean;
  selectedText(): string;
  /** Goes to the definition of the symbol at the caret. */
  goToDefinition(): void;
}

/** What a command can ask about the window when it is offered or run. */
export interface CommandContext {
  /** A project is open: its views and tool windows exist. */
  repoOpen: boolean;
  /** A review has started and can be finished. */
  reviewOpen: boolean;
  canQuit: boolean;
  /** The changeset on view has more than one commit, so there is something to review by commit. */
  manyCommits?: boolean;
  /**
   * The tab commands apply to: the active one, or the one a tab's context menu was opened on. `path`
   * is what the tab shows: a file's path, or for a log tab the check run it is the log of.
   */
  tab?: { kind: TabKind; path?: string };
  /** The active tab shows hunks to step through: a diff view, or a changed file's text. */
  diffView: boolean;
  /** Index of the current hunk in the active view, -1 for none. */
  hunk: number;
  /** The review view shows one commit of a changeset that is reviewed by commit. */
  byCommit?: boolean;
  /** The editor with the focus, if one has it. */
  editor?: FocusedEditor;
  /** The file or folder a context menu was opened on. */
  target?: { path: string; dir: boolean };
  /** The comment thread a context menu was opened on. */
  thread?: Thread;
}

const NONE: CommandContext = { repoOpen: false, reviewOpen: false, canQuit: false, diffView: false, hunk: -1 };

export const commands = new CommandRegistry<CommandContext>(NONE, isMac ? "Meta" : "Ctrl", bindingSignal);

/**
 * The items of a menu description for the context of now plus `extra` (a context menu's subject),
 * as the window draws them. Choosing one runs `commands.run(id, context)` with the same context.
 */
export function menuFor(spec: MenuSpec, extra?: Partial<CommandContext>) {
  const context = commands.context(extra);
  return { items: resolveMenu(commands, spec, context), context };
}
