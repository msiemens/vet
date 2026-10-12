// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it.

import { tick } from "svelte";
import type { CheckLog } from "../../generated/CheckLog";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { CheckRun } from "../../generated/CheckRun";
import type { Side } from "../../generated/Side";
import type { TextSpan } from "../../generated/TextSpan";
import { isAbort } from "../../lib/abort";
import type { LocationRow } from "../../generated/LocationRow";
import { nextPaint } from "../../lib/nextPaint";
import { platformName } from "../../lib/platformName";
import type { Backend } from "../../state/backend";
import type { Changeset } from "../../state/changeset.svelte";
import { contextMenus } from "../../state/contextMenu.svelte";
import { openUrl } from "../../state/host";
import { platform } from "../../state/platform.svelte";
import { locationPicker } from "../../state/locationPicker.svelte";
import { LocationSearch } from "../../state/locationSearch";
import type { FileLayout, Frame, ViewLayout } from "../../state/reviewModel";
import { semantic } from "../../state/semantic.svelte";
import { serverList } from "../../state/serverList.svelte";
import { languageOf, withoutImplementations } from "../../state/serverStatus";
import { session } from "../../state/session.svelte";
import { type Shown, isCurrent } from "../../state/shown";
import { threads } from "../../state/threads.svelte";
import { rangeKey } from "../../state/timelineRange";
import type { Thread } from "../../generated/Thread";
import { SUBMIT_KEY } from "../../lib/composer";
import { errorText } from "../../lib/errorText";
import { ChangesetView, type Listeners, type PanePosition, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import { LogView } from "./editor/LogView";
import { type ThreadActions, buildCard } from "./editor/threadCard";
import type { ThreadListeners } from "./editor/threadLayer";
import { bindRevision, focusedEditor } from "./editor/semantic";
import type { SoftWrap } from "../../lib/softWrap";
import type { Mode, ViewMode } from "../../lib/viewMode";
import { type CommandContext, commands } from "../../state/commands";
import { type TextActions, TextView } from "./editor/TextView";
import { type FilePositions, type SavedView, ViewMemory, restoredTabs } from "./changesetViews";
import { keyId } from "../../state/changesetKey";
import { threadMenu } from "../comments";
import { type HunkPlace, stepFlagged } from "./flaggedHunks";
import { provideActions } from "./reviewActions";
import type { ViewActions } from "./reviewCommands";
import {
  CHANGES,
  type TabId,
  fileTab,
  hasDiff,
  kindOf,
  logTab,
  modesOf,
  pathOf,
  settleMode,
  subjectOf,
} from "./tabModel";
import { Tabs } from "./Tabs.svelte";

/** Space types in a field and presses a focused control; it marks the current item only when none of those has the focus. */
const takesSpace = (el: Element | null) =>
  !!el?.closest("button, a, input, textarea, select, [contenteditable], [tabindex], [role=menuitem], [role=tab]");

/** What the controller reads of the open changeset and asks of it. */
export type ChangesetSource = Pick<
  Changeset,
  | "baseChanged"
  | "currentShown"
  | "currentLayout"
  | "shownCommit"
  | "stepCommit"
  | "subscribe"
  | "state"
  | "toggleHunk"
  | "toggleFlag"
  | "toggleItem"
  | "toggleFileSeen"
  | "reconfirmFile"
  | "baseDiff"
  | "commentTarget"
  | "refresh"
  | "reportFailure"
>;

const NO_HOVER = { text: "", code: [] };

/** The tabs and positions are saved this long after the last change. */
const SAVE_AFTER_MS = 400;

/** Said when a mark or a comment is asked for in a range that does not end at the latest version. */
export const READ_ONLY_NOTE = "This range does not end at the latest version: marks and comments are off";

export class ReviewController {
  readonly tabs = new Tabs("unified");
  /** The diff view of each attached tab; a file tab has none while its file is not in the changeset. */
  private views = new Map<TabId, ChangesetView>();
  /** The text view of each file tab that showed its text; built when the tab first shows it. */
  private texts = new Map<TabId, TextView>();
  /** The logs fetched for the log tabs, which are built from them whenever their pane attaches. */
  private logs = new Map<TabId, CheckLog>();
  /** The view of each attached log tab. */
  private logViews = new Map<TabId, LogView>();
  /** The scroll root of each attached tab's diff (or log) view. */
  private roots = new Map<TabId, HTMLElement>();
  /** The scroll root of each attached file tab's text view. */
  private textRoots = new Map<TabId, HTMLElement>();
  /** The diff mode chosen last, which a diff tab opens in. */
  private diffMode: Mode = "unified";
  /** Which files the views wrap the long lines of. */
  softWrap: SoftWrap = "prose";
  private hooks: ViewHooks;
  /** Show Usages and Go to Implementation: goes to the one place found, or lists the places in the popup. */
  private search = new LocationSearch({
    picker: locationPicker,
    notice: (text) => (session.notice = text),
    open: (row, side) => void this.openRow(row, side),
    isAbort,
    fail: (error, revision) => this.changeset.reportFailure(error, revision),
    restartServer: (language) => serverList.restart(language),
  });
  /** The changeset on view, by `keyId`; unknown before the first manifest. */
  private key?: string;
  /** The range the views show, if they show one. */
  private shown?: { from: number; to: number; revision: number };
  /** The files the Changes tab was limited to when it was built, if it was. */
  private filtered?: ReadonlySet<string>;
  /** The commit the views show, if they show one. */
  private shownCommit?: number;
  /** What each changeset of the repository had open when it was last on view, for when it comes back. */
  private memory?: ViewMemory;
  /** Scroll positions the next builds start at, instead of carrying the old views' over. */
  private restore?: { scroll: Map<TabId, number> };
  /** Where file tabs' panes are put when they are built: the positions of the changeset just brought back. */
  private pending = new Map<TabId, FilePositions>();
  private saving?: ReturnType<typeof setTimeout>;
  /** The Changes tab of the views built last going to its first hunk. */
  private arriving: Promise<void> = Promise.resolve();

  constructor(
    readonly backend: Backend,
    private changeset: ChangesetSource,
    /** Context menu on a file's header in a view. */
    private fileMenu: (path: string, x: number, y: number) => void,
  ) {
    // The semantic layer reads the changeset's snapshots: in a range only the new side of the latest version is one.
    this.hooks = {
      hover: (revision, side, path, line, col, signal) =>
        this.analysable(side) ? semantic.hover(revision, side, path, line, col, signal) : Promise.resolve(NO_HOVER),
      definition: (revision, side, path, line, col) => {
        if (this.analysable(side)) void this.goToDefinition(revision, side, path, line, col);
      },
      locations: (revision, kind, side, path, at, subject) => {
        if (!this.analysable(side)) return;
        const backend = this.backend;
        void this.search.start({
          kind,
          symbol: subject.name,
          anchor: subject.anchor,
          side,
          revision,
          ask: (signal) =>
            kind === "usages"
              ? backend.usages(revision, side, path, at.line, at.col, signal)
              : backend.implementations(revision, side, path, at.line, at.col, signal),
        });
      },
      diagnostics: (revision, path) =>
        this.analysable("target") ? semantic.diagnostics(revision, path) : Promise.resolve([]),
    };
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.subscribe((paths) => this.showReview(paths));
    const offThreads = threads.onChange(() => this.showThreads());
    const offCommands = provideActions(this.viewActions());
    const offContext = commands.provide(() => this.facts());
    this.unsubscribe = () => {
      offReview();
      offThreads();
      offCommands();
      offContext();
    };
    // A server installed or reconfigured elsewhere may answer differently now.
    this.unsubscribeEvents = backend.onEvent((event) => {
      if (event.type !== "serversChanged" || event.project !== backend.project) return;
      semantic.invalidate();
      for (const view of this.views.values()) view.refreshDiagnostics();
      for (const text of this.texts.values()) text.refreshDiagnostics();
    });
  }

  private unsubscribe: () => void;
  private unsubscribeEvents: () => void;

  /** Detaches the controller from the window and the store, and tears down the views. */
  destroy() {
    this.search.cancel();
    clearTimeout(this.saving);
    this.saveView();
    this.unsubscribe();
    this.unsubscribeEvents();
    for (const view of this.views.values()) view.destroy();
    this.views.clear();
    for (const text of this.texts.values()) text.destroy();
    this.texts.clear();
    for (const log of this.logViews.values()) log.destroy();
    this.logViews.clear();
  }

  /** The files of the view on screen. */
  private get files(): readonly FileLayout[] {
    return this.changeset.currentLayout()?.files ?? [];
  }

  /** The view shows a range that does not end at the latest version: nothing can be marked or commented. */
  private get readOnly(): boolean {
    return this.changeset.currentLayout()?.interdiff?.markable === false;
  }

  /** Whether the semantic layer has `side` of what the view shows: not in a commit, whose snapshots are not the changeset's. */
  private analysable(side: Side): boolean {
    if (this.changeset.shownCommit !== undefined) return false;
    const range = this.changeset.currentLayout()?.interdiff;
    return !range || (range.markable && side === "target");
  }

  /** The threads the views show: those of the changeset, and none in a commit, where they would sit in the wrong place. */
  private shownThreads(): Thread[] {
    return this.changeset.shownCommit === undefined ? threads.items : [];
  }

  /** Runs a request that marks or comments, unless the view is read-only: then it says so and `refuse` answers. */
  private async editing<T>(request: () => Promise<T>, refuse: () => T): Promise<T> {
    if (!this.readOnly) return request();
    session.notice = READ_ONLY_NOTE;
    return refuse();
  }

  /** The Changes tab's scroll root. */
  get changesRoot(): HTMLElement {
    const root = this.roots.get(CHANGES);
    if (!root) throw new Error("the Changes tab is not mounted");
    return root;
  }

  /** The Changes tab's view; throws before the first `open`. */
  get changes(): ChangesetView {
    const view = this.views.get(CHANGES);
    if (!view) throw new Error("no changeset view is mounted");
    return view;
  }

  /** The diff view of the focused tab, unless the tab shows its file's text. */
  private get active(): ChangesetView | undefined {
    return this.activeMode === "file" ? undefined : this.views.get(this.tabs.active);
  }

  /** The text of the focused tab, if it shows it and the file is a changed one with hunks to step through. */
  private get activeText(): TextView | undefined {
    const text = this.activeMode === "file" ? this.texts.get(this.tabs.active) : undefined;
    return text?.hasHunks ? text : undefined;
  }

  /** What the active tab shows; none for a log. */
  get activeMode(): ViewMode | undefined {
    return this.tabs.modes.get(this.tabs.active);
  }

  private isChanged(path: string): boolean {
    return this.files.some((f) => f.path === path);
  }

  /** The facts about the review screen that commands read. */
  private facts(): Partial<CommandContext> {
    const id = this.tabs.active;
    const path = pathOf(id);
    const view = this.active;
    const text = this.activeText;
    const editor = focusedEditor(document.activeElement);
    const file = editor?.path ?? path;
    return {
      tab: { kind: kindOf(id), path: subjectOf(id) },
      tabCount: this.tabs.open.length,
      viewMode: this.activeMode,
      viewModes: modesOf(kindOf(id), path !== undefined && this.isChanged(path)),
      diffView: view !== undefined || text !== undefined,
      byCommit: this.changeset.shownCommit !== undefined,
      hunk: view?.current ?? text?.current ?? -1,
      editor,
      noImplementations: editor && withoutImplementations(editor.path, serverList.servers),
      serverLanguage: file === undefined ? undefined : languageOf(file, serverList.servers),
    };
  }

  private viewActions(): ViewActions {
    return {
      step: (delta) => this.stepHunk(delta),
      toggleSeen: () => {
        const text = this.activeText;
        if (text) return text.toggleCurrent();
        const view = this.active;
        return !!view && this.toggleBySpace(view);
      },
      toggleFlag: () => {
        const text = this.activeText;
        if (text) return text.toggleFlagCurrent();
        return this.active?.toggleFlagCurrent() !== undefined;
      },
      stepFlag: (delta) => this.stepFlagged(delta > 0 ? 1 : -1),
      firstFlagged: () => this.goToFlagged(this.flaggedFrom(undefined, 1)),
      reconfirm: () => {
        const text = this.activeText;
        if (text) return text.reconfirmCurrent();
        const done = this.active?.reconfirmCurrent();
        return done !== undefined;
      },
      comment: () => this.activeText?.commentHere() ?? this.active?.commentHere() ?? false,
      showCard: () => this.activeText?.showCaretCard() ?? false,
      jumpToSource: () => {
        const caret = this.active?.caret();
        if (!caret) return false;
        void this.openSource(caret.path, caret.line);
        return true;
      },
      stepCommit: (delta) => this.changeset.stepCommit(delta),
      setViewMode: (mode) => void this.setViewMode(mode),
    };
  }

  /**
   * Space in a diff view: with an editor focused it flips the hunk at the caret; otherwise the
   * current stop, unless a field or control has the focus and takes the key. Answers whether it acted.
   */
  private toggleBySpace(view: ChangesetView): boolean {
    if (view.toggleAtCaret()) return true;
    if (view.current < 0 || takesSpace(document.activeElement)) return false;
    void view.toggleCurrent();
    return true;
  }

  /** The hunk the focused tab is at: the current one of its view, or of its text. */
  private hunkPlace(): HunkPlace | undefined {
    const path = pathOf(this.tabs.active);
    const text = this.activeText;
    if (text && path !== undefined) return { path, hunk: text.current };
    return this.active?.currentHunk();
  }

  /** The hunk flagged after or before `from` over the files of the view, as they are flagged now. */
  private flaggedFrom(from: HunkPlace | undefined, delta: 1 | -1): HunkPlace | undefined {
    const paths = this.changeset.currentLayout()?.paths ?? [];
    return stepFlagged(paths, (path) => this.changeset.state(path)?.review.flagged, from, delta);
  }

  /** Goes to the flagged hunk after or before the current one, across files; says so when there is none. */
  private stepFlagged(delta: 1 | -1): boolean {
    if (this.goToFlagged(this.flaggedFrom(this.hunkPlace(), delta))) return true;
    session.notice = delta > 0 ? "No further flagged hunk" : "No earlier flagged hunk";
    return false;
  }

  /** Shows a hunk in the focused tab, or in the tab of its file when that is not the tab's; answers whether there was one. */
  private goToFlagged(to: HunkPlace | undefined): boolean {
    if (!to) return false;
    const id = this.tabs.active;
    void (async () => {
      // New views go to their first hunk first; a later jump must not be undone by it.
      await this.arriving.catch(() => undefined);
      if (id === CHANGES) {
        await this.views.get(CHANGES)?.goToFileHunk(to.path, to.hunk);
      } else {
        // The file's tab, in the mode of the tab the step started from.
        const mode = await this.showFile(to.path, this.tabs.modes.get(id));
        const tab = fileTab(to.path);
        if (mode === "file") this.texts.get(tab)?.goToHunk(to.hunk);
        else await this.views.get(tab)?.goToFileHunk(to.path, to.hunk);
      }
    })();
    return true;
  }

  /** The file of the focused file or text tab; none for the Changes tab. */
  get activePath(): string | undefined {
    return pathOf(this.tabs.active);
  }

  /** Shows a place the popup listed: in its file's tab, or says where it is when it is outside the repository. */
  private async openRow(row: LocationRow, side: Side) {
    if (row.external) {
      session.notice = `Outside the repository: ${row.path}`;
      return;
    }
    const line = row.line - 1;
    const col = row.col - 1;
    const width = row.matchEnd - row.matchStart;
    await this.openLocation(row.path, line, side, { start: { line, col }, end: { line, col: col + width } });
  }

  /** The definition of the symbol at a position of `side`'s text, shown by `openLocation`. */
  private async goToDefinition(revision: number, side: Side, path: string, line: number, col: number) {
    const found = await semantic.definition(revision, side, path, line, col);
    const location = found?.[0];
    if (!location) return;
    if (location.external) {
      session.notice = `Defined outside the repository: ${location.path}`;
      return;
    }
    await this.openLocation(location.path, location.span.start.line, side, location.span);
  }

  /**
   * Shows a 0-based `line` of `side`'s text of the file at `path`, marking `span` if given, in the
   * file's tab. A changed file's tab keeps its mode (a new one opens in the diff mode last chosen);
   * an unchanged file (the same text on both sides) has its text. A base-side location lands in
   * the base content: the base editor of a side-by-side view, or the target line of the unified
   * view where the deleted lines are drawn. Without a span a base line has no place in the diff
   * views yet, so the diff opens at its start. The text has no base side: a tab showing it
   * changes to the diff for such a location.
   */
  async openLocation(path: string, line: number | undefined, side: Side, span?: TextSpan) {
    // A base path is the file's old path if it was renamed.
    const file = this.files.find((f) => (side === "base" ? (f.oldPath ?? f.path) : f.path) === path);
    const target = file?.path ?? path;
    const showsText = this.tabs.modes.get(fileTab(target)) === "file";
    const mode = await this.showFile(target, side === "base" && showsText ? this.diffMode : undefined);
    if (mode === "file") {
      const text = this.texts.get(fileTab(target));
      if (span) text?.goToSpan(span);
      else if (line !== undefined) text?.goToLine(line);
    } else {
      const view = this.views.get(fileTab(target));
      if (span) await view?.goToSpan(0, side, span);
      else if (side === "target" && line !== undefined) await view?.goToLine(0, line);
    }
  }

  /**
   * Focuses the file's one tab, opening it if needed, showing `want` if the file has that mode
   * (else the tab's mode, or for a new tab the diff mode last chosen), with the view built.
   * Answers the mode.
   */
  private async showFile(path: string, want?: ViewMode): Promise<ViewMode> {
    const id = fileTab(path);
    const mode = settleMode({
      changed: this.isChanged(path),
      want,
      current: this.tabs.modes.get(id),
      last: this.diffMode,
    });
    this.tabs.showFile(path, mode);
    await tick(); // a new tab's pane attaches and builds its view
    if (mode === "file") this.ensureText(id);
    else this.syncDiffMode(id);
    return mode;
  }

  /** Draws the tab's diff view in the tab's diff mode. */
  private syncDiffMode(id: TabId) {
    const mode = this.tabs.modes.get(id);
    const view = this.views.get(id);
    if (view && mode !== undefined && mode !== "file" && view.mode !== mode) view.setMode(mode);
  }

  /** Builds the tab's text view if the tab has its root and no view yet. */
  private ensureText(id: TabId) {
    const root = this.textRoots.get(id);
    const layout = this.changeset.currentLayout();
    if (root && layout && !this.texts.has(id)) this.buildText(id, root, layout);
  }

  /**
   * Shows a changed symbol where it is reviewed: its hunk in the Changes tab, made the current one;
   * else the file's tab at the symbol (its text tab when the file is unchanged).
   */
  async revealSymbol({ symbol, change }: ChangedSymbol) {
    const side: Side = change === "removed" ? "base" : "target";
    this.tabs.active = CHANGES;
    if (await this.views.get(CHANGES)?.goToSymbol(symbol.id.path, side, symbol.span)) return;
    await this.openLocation(symbol.id.path, symbol.span.start.line, side, symbol.span);
  }

  /** Jump to Source for a changed symbol: its file's text at the symbol. A removed symbol has no text, so it is shown where it is reviewed. */
  async jumpToSymbol(changed: ChangedSymbol) {
    if (changed.change === "removed") await this.revealSymbol(changed);
    else await this.openSource(changed.symbol.id.path, changed.symbol.span.start.line);
  }

  /** Scrolls the Changes tab to a changed file; answers whether `path` is one. */
  async revealInChanges(path: string): Promise<boolean> {
    // The Changes tab may show only some of the changed files.
    const index = this.changes.indexOf(path);
    if (index < 0) return false;
    this.tabs.active = CHANGES;
    await this.changes.goToFile(index);
    return true;
  }

  private listenersFor(id: TabId, built: Shown): Listeners {
    return {
      pinned: id === CHANGES ? (path) => (this.tabs.changesFile = path) : undefined,
      toggleHunk: (shown, path, hunk) =>
        this.editing(
          () => this.changeset.toggleHunk(shown, path, hunk),
          () => undefined,
        ),
      toggleFlag: (shown, path, hunk) =>
        this.editing(
          () => this.changeset.toggleFlag(shown, path, hunk),
          () => undefined,
        ),
      toggleItem: (shown, path, kind) =>
        this.editing(
          () => this.changeset.toggleItem(shown, path, kind),
          () => undefined,
        ),
      toggleFileSeen: (shown, path) =>
        this.editing(
          () => this.changeset.toggleFileSeen(shown, path),
          () => undefined,
        ),
      reconfirmFile: (shown, path) =>
        this.editing(
          () => this.changeset.reconfirmFile(shown, path),
          () => undefined,
        ),
      // Only the changeset has a base diff to show, not a range of versions.
      baseDiff:
        built.view.type === "changeset" ? (shown, path, hunk) => this.changeset.baseDiff(shown, path, hunk) : undefined,
      fileMenu: this.fileMenu,
      openSource: (path, line) => void this.openSource(path, line),
      threads: this.threadListeners(),
    };
  }

  private threadListeners(): ThreadListeners {
    return {
      create: (shown, target, body) =>
        this.editing(
          () => threads.create(shown, target, body),
          () => false,
        ),
      commentTarget: (shown, path, at) =>
        this.editing(
          () => this.changeset.commentTarget(shown, path, at),
          () => {
            throw new Error(READ_ONLY_NOTE);
          },
        ),
      reply: (thread, body) => threads.reply(thread, body),
      setStatus: (thread, status) => threads.setStatus(thread, status),
      remove: (thread) => threads.remove(thread),
      editMessage: (message, body) => threads.editMessage(message, body),
      removeMessage: (message) => threads.removeMessage(message),
      keepEdit: (message, keep) => threads.keepEdit(message, keep),
      platformName: () => (platform.status ? platformName(platform.status.kind) : "the platform"),
      confirm: (thread) => threads.confirm(thread),
      menu: (thread, x, y) => {
        const subject = threads.items.find((t) => t.id === thread);
        if (subject) void contextMenus.open(threadMenu, x, y, { thread: subject });
      },
      openLink: (url) =>
        void openUrl(url).catch((e: unknown) => {
          session.notice = `Could not open the browser: ${errorText(e)}`;
        }),
      submitKey: commands.keyLabel(SUBMIT_KEY),
    };
  }

  /**
   * A thread as the card the editors show, for a place outside them (the Comments tool window), with
   * the same actions; `toggle` opens or folds a resolved one.
   */
  threadCard(thread: Thread, open: boolean, toggle: () => void): HTMLElement {
    const actions: ThreadActions = {
      ...this.threadListeners(),
      create: () => Promise.resolve(false),
      cancelDraft: () => undefined,
      toggle,
    };
    return buildCard({ thread, open }, actions);
  }

  /** The threads were read for the revision and the range `layout` is of. */
  private threadsFit(layout: ViewLayout): boolean {
    return threads.revision === layout.shown.revision && threads.key === rangeKey(layout.interdiff);
  }

  /**
   * Review state was written for `paths`: the views of what is on screen show it as the changeset holds
   * it now. The views of another revision or view are about to be replaced and are left alone.
   */
  private showReview(paths: ReadonlySet<string> | "all") {
    const shown = this.changeset.currentShown();
    if (!shown) return;
    const wanted = (path: string) => paths === "all" || paths.has(path);
    for (const view of this.views.values()) {
      if (!isCurrent(view.shown, shown)) continue;
      for (const path of view.paths()) {
        const state = wanted(path) ? this.changeset.state(path) : undefined;
        if (state) view.showReview(path, state);
      }
    }
    for (const [id, text] of this.texts) {
      const path = pathOf(id);
      const state = path !== undefined && wanted(path) ? this.changeset.state(path) : undefined;
      if (state && text.isFor(shown)) text.applyReview(state.review);
    }
  }

  /** Gives every view the threads, if they are of the view's revision and range. */
  private showThreads() {
    const layout = this.changeset.currentLayout();
    if (threads.revision === undefined || !layout || !this.threadsFit(layout)) return;
    for (const view of this.views.values()) view.setThreads(this.shownThreads(), threads.revision);
    for (const text of this.texts.values()) text.setThreads(this.shownThreads(), threads.revision);
  }

  /**
   * Shows a thread on code: its card in the file's tab, in the mode that tab is in, when the file
   * has a tab open; else in the Changes tab, where it is its card or its file. The changeset's
   * own threads are in no diff.
   */
  async goToThread(thread: Thread): Promise<boolean> {
    if (thread.kind === "changeset") return false;
    const id = thread.path === null ? undefined : fileTab(thread.path);
    if (id !== undefined && this.tabs.open.includes(id)) {
      const file = this.tabs.modes.get(id) === "file";
      this.tabs.active = id;
      if (file) this.ensureText(id);
      if (file ? this.texts.get(id)?.goToThread(thread) : await this.views.get(id)?.goToThread(thread)) return true;
    }
    this.tabs.active = CHANGES;
    return this.views.get(CHANGES)?.goToThread(thread) ?? false;
  }

  /** The diff mode the tab's diff view draws in: the tab's own, or while it shows its text, the one chosen last. */
  private diffModeOf(id: TabId): Mode {
    const mode = this.tabs.modes.get(id);
    return mode === undefined || mode === "file" ? this.diffMode : mode;
  }

  /** Builds a file tab's text view in its root, replacing the one it had. */
  private buildText(id: TabId, root: HTMLElement, layout: ViewLayout) {
    const path = pathOf(id);
    if (path === undefined) return;
    this.texts.get(id)?.destroy();
    const shown = this.changeset.currentShown();
    if (!shown) return;
    const file = layout.files.find((f) => f.path === path);
    const text = new TextView(
      root,
      path,
      fileSource(this.backend, shown.revision, shown.view),
      bindRevision(this.hooks, layout.shown.revision),
      this.softWrap,
      { shown, listeners: this.threadListeners() },
      file && { file, review: this.changeset, actions: this.textActions(file, shown) },
    );
    this.texts.set(id, text);
    const at = this.takePending(id, "text");
    if (at) text.showLines(at);
    if (this.threadsFit(layout)) text.setThreads(this.shownThreads(), layout.shown.revision);
  }

  /**
   * Builds the tab's diff view in its root, replacing `previous`. The scroll position and the
   * current hunk (the nearest one of the same file) carry over.
   */
  private build(id: TabId, layout: ViewLayout, previous?: ChangesetView) {
    const root = this.roots.get(id);
    if (!root || kindOf(id) === "log") return;
    const path = pathOf(id);
    const only = this.changeset.baseChanged;
    const files =
      path === undefined
        ? layout.files.filter((f) => !only || only.has(f.path))
        : layout.files.filter((f) => f.path === path);
    const restoring = this.restore?.scroll.has(id) ? this.restore : undefined;
    const anchor = restoring ? undefined : previous?.currentAnchor();
    const top = restoring ? (restoring.scroll.get(id) ?? 0) : root.scrollTop;
    if (restoring) {
      restoring.scroll.delete(id);
      if (restoring.scroll.size === 0) this.restore = undefined;
    }
    previous?.destroy();
    this.views.delete(id);
    if (path !== undefined && !files.length) return; // the file left the changeset
    const shown = this.changeset.currentShown();
    if (!shown) return;
    const view = new ChangesetView(
      root,
      shown,
      files,
      this.changeset,
      fileSource(this.backend, shown.revision, shown.view),
      this.diffModeOf(id),
      this.softWrap,
      this.hooks,
      this.listenersFor(id, shown),
      id === CHANGES, // only the Changes tab collapses unchanged regions
    );
    this.views.set(id, view);
    if (this.threadsFit(layout)) view.setThreads(this.shownThreads(), layout.shown.revision);
    root.scrollTop = top;
    view.keepCurrent(anchor);
    view.sync();
    // A file tab shows its one file at once; the Changes tab loads sections as they come near.
    // A stale revision is replaced by the next build.
    if (path === undefined) return;
    const at = this.takePending(id, "diff");
    (at ? view.showLines(at) : view.load(0)).catch(() => undefined);
  }

  /** The position a pane is put at when it is built, once. */
  private takePending(id: TabId, pane: keyof FilePositions): PanePosition | undefined {
    const positions = this.pending.get(id);
    const at = positions?.[pane];
    if (positions) {
      positions[pane] = undefined;
      if (!positions.diff && !positions.text) this.pending.delete(id);
    }
    return at;
  }

  /** The views of the tabs now, as `memory` keeps them. Where a pane has no view to ask (its tab's diff pane while the next changeset loads), the position stored for it stays. */
  private capture(key: string): SavedView {
    const before = this.memory?.get(key);
    // A range or a commit shows other files than the changeset's own: its positions are not kept.
    const plain = !this.shown && this.shownCommit === undefined;
    const panes = new Map<TabId, FilePositions>();
    for (const id of this.tabs.open) {
      if (pathOf(id) === undefined) continue;
      const old = before?.panes.get(id);
      const live = plain && {
        diff: this.views.get(id)?.lines() ?? this.pending.get(id)?.diff,
        text: this.texts.get(id)?.lines() ?? this.pending.get(id)?.text,
      };
      const kept = live ? { diff: live.diff ?? old?.diff, text: live.text ?? old?.text } : old;
      if (kept && (kept.diff || kept.text)) panes.set(id, kept);
    }
    const changes = this.views.get(CHANGES);
    return {
      open: [...this.tabs.open],
      active: this.tabs.active,
      modes: new Map(this.tabs.modes),
      panes,
      changes: plain ? (changes?.position() ?? before?.changes) : before?.changes,
      anchor: plain ? (changes?.currentAnchor() ?? before?.anchor) : before?.anchor,
    };
  }

  /** Remembers the tabs and where the panes are for the changeset on view. */
  private saveView() {
    const repo = session.repo;
    if (repo === undefined || this.key === undefined) return;
    this.memory ??= new ViewMemory(repo);
    this.memory.set(this.key, this.capture(this.key));
  }

  /** The tabs or a pane changed: the views are saved once the changes pause. */
  scheduleSave = () => {
    clearTimeout(this.saving);
    this.saving = setTimeout(() => this.saveView(), SAVE_AFTER_MS);
  };

  /** A file tab's text pane has its scroll root; returns the function that detaches it. The text is built once the tab shows it. */
  attachText(id: TabId, root: HTMLElement): () => void {
    this.textRoots.set(id, root);
    // The text view's own scroller does not bubble its scrolls.
    root.addEventListener("scroll", this.scheduleSave, { passive: true, capture: true });
    if (this.tabs.modes.get(id) === "file") this.ensureText(id);
    return () => {
      root.removeEventListener("scroll", this.scheduleSave, { capture: true });
      this.texts.get(id)?.destroy();
      this.texts.delete(id);
      this.textRoots.delete(id);
    };
  }

  /** A tab's pane has its scroll root (for a file tab, that of its diff); returns the function that detaches it. */
  attach(id: TabId, root: HTMLElement): () => void {
    this.roots.set(id, root);
    // Registered before the view is, so it hears the scroll that the view's own listener stops.
    root.addEventListener("scroll", this.scheduleSave, { passive: true });
    // The Changes tab is built by `open`, which fetches the manifest first.
    const layout = this.changeset.currentLayout();
    if (kindOf(id) === "log") this.buildLog(id, root);
    else if (id !== CHANGES && layout) this.build(id, layout);
    return () => {
      root.removeEventListener("scroll", this.scheduleSave);
      this.views.get(id)?.destroy();
      this.views.delete(id);
      this.logViews.get(id)?.destroy();
      this.logViews.delete(id);
      this.roots.delete(id);
    };
  }

  private buildLog(id: TabId, root: HTMLElement) {
    const log = this.logs.get(id);
    if (log) this.logViews.set(id, new LogView(root, log.text, log.failure?.line));
  }

  /**
   * Shows the log of a failed check run in a tab of its own, scrolled to the line where the job
   * failed. The log is fetched the first time; a log that cannot be had is reported and opens no tab.
   */
  async openLog(run: CheckRun) {
    const id = logTab(run.id);
    let log = this.logs.get(id);
    if (!log) {
      try {
        log = await this.backend.checkLog(run.id);
      } catch (error) {
        session.notice = `Could not load the log of ${run.name}: ${errorText(error)}`;
        return;
      }
      this.logs.set(id, log);
    }
    const step = log.failure?.step;
    this.tabs.showLog(run.id, {
      title: `${run.name} · log`,
      hint: step ? `${run.name}: the step "${step}" failed` : undefined,
    });
    await tick(); // a new tab's pane attaches and builds its view
    this.logViews.get(id)?.goToFailure();
  }

  /**
   * Shows the active tab in the view mode: the Changes tab or a changed file's diff drawn that way,
   * or a file's text. The tab stays the same one; a file's other view keeps its own caret and
   * scroll, and the caret's line carries over to the view that comes up. Does nothing in a tab
   * that does not offer the mode.
   */
  async setViewMode(mode: ViewMode) {
    const id = this.tabs.active;
    const path = pathOf(id);
    if (!modesOf(kindOf(id), path !== undefined && this.isChanged(path)).includes(mode)) return;
    const from = this.tabs.modes.get(id);
    if (mode !== "file") this.diffMode = mode;
    if (from === mode) return;
    if (path === undefined) {
      this.tabs.modes.set(id, mode);
      this.syncDiffMode(id);
      return;
    }
    const line = from === "file" ? this.texts.get(id)?.caretLine() : this.views.get(id)?.caretLine();
    this.tabs.modes.set(id, mode);
    if (mode === "file") {
      this.ensureText(id);
      if (line !== undefined) this.texts.get(id)?.goToLine(line);
    } else {
      this.syncDiffMode(id);
      if (from === "file" && line !== undefined) await this.views.get(id)?.goToLine(0, line);
    }
  }

  /**
   * Jump to Source: the file's tab shows its text, at a 0-based target `line` if given. The tab
   * is opened in that mode if the file has none.
   */
  async openSource(path: string, line?: number) {
    await this.showFile(path, "file");
    if (line !== undefined) this.texts.get(fileTab(path))?.goToLine(line);
  }

  /** What the file's text tab offers for the file's hunks. */
  private textActions(file: FileLayout, shown: Shown): TextActions {
    return {
      toggleSeen: (hunk) =>
        void this.editing(
          () => this.changeset.toggleHunk(shown, file.path, hunk),
          () => undefined,
        ),
      toggleFlag: (hunk) =>
        void this.editing(
          () => this.changeset.toggleFlag(shown, file.path, hunk),
          () => undefined,
        ),
    };
  }

  /** Renders `layout` in every tab, keeping scroll positions and current hunks (a new revision replaces the views). */
  show(layout: ViewLayout) {
    for (const id of this.roots.keys()) this.build(id, layout, this.views.get(id));
    // A text view is built where the tab shows it or showed it.
    for (const [id, root] of this.textRoots) {
      if (this.texts.has(id) || this.tabs.modes.get(id) === "file") this.buildText(id, root, layout);
    }
  }

  /**
   * A refetched manifest may be of a newer revision than the views; rebuild them then. A manifest of
   * another changeset first remembers the tabs and positions of the one that was on view, and
   * brings back those of the new one, if it was on view before.
   */
  update({ layout, facts }: Frame) {
    const key = keyId(facts.target.changeset);
    const switching = this.key !== undefined && key !== this.key;
    const paths = new Set(layout.paths);
    const brought = switching ? this.switchTo(key, paths) : undefined;
    if (!switching) this.tabs.dropStale((path) => paths.has(path));
    this.key = key;
    const commitChanged = this.commitChanged();
    const rangeChanged = this.rangeChanged(layout) || commitChanged;
    const filterChanged = this.filtered !== this.changeset.baseChanged;
    this.filtered = this.changeset.baseChanged;
    const view = this.views.get(CHANGES);
    if (!view || (!switching && view.revision === layout.shown.revision && !rangeChanged && !filterChanged)) return;
    // Another range is another set of files: its views start at the top. So does the Changes tab when
    // it shows other files.
    if ((rangeChanged || filterChanged) && !switching) {
      const tabs = this.tabs.open.filter((tab) => hasDiff(tab) && (rangeChanged || tab === CHANGES));
      this.restore = { scroll: new Map(tabs.map((tab) => [tab, 0])) };
    }
    this.show(layout);
    const restored = switching && this.restoreChanges(brought, layout);
    if (!restored && (switching || rangeChanged || filterChanged)) this.arriving = this.changes.goToHunk(0);
    // The new views have loaded nothing; a newer revision replaces this load.
    this.changes.loadAll().catch(() => undefined);
  }

  /**
   * Whether `layout` shows another range than the last one did: no range and a range differ, and
   * so do ranges that start at another version. A live range whose end moved with a new revision
   * is the same range.
   */
  private rangeChanged(layout: ViewLayout): boolean {
    const now = layout.interdiff;
    const was = this.shown;
    this.shown = now && { from: now.from, to: now.to, revision: layout.shown.revision };
    if (!now || !was) return !now !== !was;
    return now.from !== was.from || (now.to !== was.to && layout.shown.revision === was.revision);
  }

  /** Whether the views show another commit than they did, or one where there was none or none where there was one. */
  private commitChanged(): boolean {
    const now = this.changeset.shownCommit;
    const was = this.shownCommit;
    this.shownCommit = now;
    return now !== was;
  }

  /** Remembers the changeset on view and brings back the tabs of `next`; answers what was remembered of it. */
  private switchTo(next: string, paths: ReadonlySet<string>): SavedView | undefined {
    clearTimeout(this.saving);
    this.saveView();
    return this.bringBack(next, paths);
  }

  /** Sets the tabs the changeset `key` had, and what its panes are put at when they are built. */
  private bringBack(key: string, paths: ReadonlySet<string>): SavedView | undefined {
    const repo = session.repo;
    if (repo !== undefined) this.memory ??= new ViewMemory(repo);
    const saved = this.memory?.get(key);
    const { open, active, modes } = restoredTabs(
      saved,
      (path) => paths.has(path),
      this.diffMode,
      (tab) => this.tabs.logs.has(tab),
    );
    this.tabs.modes.clear();
    for (const [id, mode] of modes) this.tabs.modes.set(id, mode);
    this.tabs.open = open;
    this.tabs.active = active;
    this.pending.clear();
    for (const id of open) {
      const at = saved?.panes.get(id);
      if (at) this.pending.set(id, { ...at });
    }
    // Every diff view starts at the top; a remembered position is put on it once it is built.
    this.restore = { scroll: new Map(open.filter(hasDiff).map((tab) => [tab, 0])) };
    return saved;
  }

  /** Puts the Changes tab where `saved` left it and makes its hunk the current one; answers whether the position was found. */
  private restoreChanges(saved: SavedView | undefined, layout: ViewLayout): boolean {
    const view = this.views.get(CHANGES);
    if (!view || !saved?.changes || layout.interdiff || this.changeset.shownCommit !== undefined) return false;
    if (!view.restore(saved.changes)) return false;
    view.keepCurrent(saved.anchor);
    return true;
  }

  /** Fetches the manifest and renders it, answering whether it did (not when the project was closed); with `prefetch`, then loads the rest of the Changes tab once its first hunk is painted. */
  async open(prefetch: boolean): Promise<boolean> {
    this.views.get(CHANGES)?.destroy();
    this.views.delete(CHANGES);
    const frame = await this.changeset.refresh();
    if (!frame) return false; // the project was closed meanwhile
    const { layout, facts } = frame;
    this.key = keyId(facts.target.changeset);
    const saved = this.bringBack(this.key, new Set(layout.paths));
    this.rangeChanged(layout);
    this.filtered = this.changeset.baseChanged;
    this.commitChanged();
    this.show(layout);
    const view = this.changes;
    if (!this.restoreChanges(saved, layout)) await view.goToHunk(0);
    await nextPaint();
    if (prefetch) await view.loadAll();
    return true;
  }

  /** Builds the views anew with the setting, keeping their scroll positions and current hunks. */
  setSoftWrap(wrap: SoftWrap) {
    if (wrap === this.softWrap) return;
    this.softWrap = wrap;
    const layout = this.changeset.currentLayout();
    if (layout) this.show(layout);
  }

  /** Goes to the hunk `delta` away from the current one in the focused tab. */
  stepHunk(delta: number) {
    const text = this.activeText;
    if (text) {
      text.step(delta);
      return;
    }
    const view = this.active;
    if (!view) return;
    // By commit, the next hunk after a commit's last one is the next commit's first.
    const last = this.tabs.active === CHANGES && delta > 0 && view.current >= view.stopCount - 1;
    if (last && this.changeset.stepCommit(1)) return;
    void view.goToHunk(view.current + delta);
  }
}
