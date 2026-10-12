// The window's one command registry, and the facts commands read to decide whether they apply.
// Features register their commands with it and contribute the facts they own; the key handler,
// the menus and the tooltips read it.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { Side } from "../generated/Side";
import type { Thread } from "../generated/Thread";
import { CommandRegistry } from "../lib/commands";
import { type MenuSpec, resolveMenu } from "../lib/commandMenu";
import type { ViewMode } from "../lib/viewMode";
import { bindingSignal } from "./bindingSignal.svelte";
import { isMac } from "./host";

export type TabKind = "changes" | "file" | "log";

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
  /** Goes to the implementation of the symbol at the caret, or lists them. */
  goToImplementation(): void;
  /** Lists the usages of the symbol at the caret, or goes to the one there is. */
  showUsages(): void;
}

/** What a command can ask about the window when it is offered or run. */
export interface CommandContext {
  /** A project is open: its views and tool windows exist. */
  repoOpen: boolean;
  /** A review has started and can be finished. */
  reviewOpen: boolean;
  canQuit: boolean;
  /** The queue screen covers the review, whose views and tabs stay mounted behind it. */
  queueShown?: boolean;
  /** A changeset is on view. */
  changesetOpen?: boolean;
  /** The review view shows a range of versions rather than the changeset. */
  rangeShown?: boolean;
  /** Hunks seen against an old base wait to be re-confirmed. */
  reconfirmable?: boolean;
  /** A pull request is on view. */
  pullRequest?: boolean;
  /** How many comments and staged votes wait to be sent to the platform. */
  outgoing?: number;
  /** The pull request's platform keeps the vote apart from the comments, so the viewer can stage it alone. */
  voteSeparate?: boolean;
  /** Why a vote cannot be staged now, if it cannot. */
  voteBlock?: string;
  /** The changeset on view has more than one commit, so there is something to review by commit. */
  manyCommits?: boolean;
  /**
   * The tab commands apply to: the active one, or the one a tab's context menu was opened on. `path`
   * is what the tab shows: a file's path, or for a log tab the check run it is the log of.
   */
  tab?: { kind: TabKind; path?: string };
  /** How many tabs are open. */
  tabCount?: number;
  /** What the active tab shows: a diff in a mode, or a file's text; none for a log. */
  viewMode?: ViewMode;
  /** The view modes the active tab offers. */
  viewModes?: readonly ViewMode[];
  /** The active tab shows hunks to step through: a diff view, or a changed file's text. */
  diffView: boolean;
  /** Index of the current hunk in the active view, -1 for none. */
  hunk: number;
  /** The review view shows one commit of a changeset that is reviewed by commit. */
  byCommit?: boolean;
  /** The editor with the focus, if one has it. */
  editor?: FocusedEditor;
  /** The name of the focused editor's language server when it has said it offers no implementations. */
  noImplementations?: string;
  /** The language of the active file's server, if some language serves its type of file. */
  serverLanguage?: string;
  /** The file or folder a context menu was opened on. */
  target?: { path: string; dir: boolean };
  /** The queue row a context menu was opened on; `url` is its pull request's page. */
  row?: { key: ChangesetKey; url?: string };
  /** The recent project a context menu was opened on. */
  recentProject?: { path: string };
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
