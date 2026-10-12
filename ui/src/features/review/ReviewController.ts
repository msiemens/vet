// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it.

import { tick } from "svelte";
import type { CheckLog } from "../../generated/CheckLog";
import type { ChangedSymbol } from "../../generated/ChangedSymbol";
import type { CheckRun } from "../../generated/CheckRun";
import type { FileChange } from "../../generated/FileChange";
import type { Manifest } from "../../generated/Manifest";
import type { Side } from "../../generated/Side";
import type { TextSpan } from "../../generated/TextSpan";
import { nextPaint } from "../../lib/nextPaint";
import { platformName } from "../../lib/platformName";
import type { Backend } from "../../state/backend";
import type { Changeset } from "../../state/changeset.svelte";
import { contextMenus } from "../../state/contextMenu.svelte";
import { openUrl } from "../../state/host";
import { platform } from "../../state/platform.svelte";
import { semantic } from "../../state/semantic.svelte";
import { session } from "../../state/session.svelte";
import { threads } from "../../state/threads.svelte";
import { rangeKey } from "../../state/timelineRange";
import type { Thread } from "../../generated/Thread";
import { SUBMIT_KEY } from "../../lib/composer";
import { errorText } from "../../lib/errorText";
import { ChangesetView, type Listeners, type ScrollPosition, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import { LogView } from "./editor/LogView";
import { type ThreadActions, buildCard } from "./editor/threadCard";
import type { ThreadListeners } from "./editor/threadLayer";
import { bindRevision, focusedEditor } from "./editor/semantic";
import type { SoftWrap } from "../../lib/softWrap";
import type { Mode, ViewMode } from "../../lib/viewMode";
import { type CommandContext, commands } from "../../state/commands";
import { type TextActions, TextView } from "./editor/TextView";
import { type SavedView, restoredTabs } from "./changesetViews";
import { keyId } from "../../state/changesetKey";
import { threadMenu } from "../comments";
import { provideActions } from "./reviewActions";
import type { ViewActions } from "./reviewCommands";
import { readPosition, writePosition } from "./savedPosition";
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
  | "files"
  | "baseChanged"
  | "view"
  | "shownCommit"
  | "stepCommit"
  | "onReview"
  | "toggleHunk"
  | "toggleItem"
  | "toggleFileSeen"
  | "reconfirmFile"
  | "baseDiff"
  | "currentManifest"
  | "refresh"
>;

const NO_HOVER = { text: "", code: [] };

/** The Changes tab's position is saved this long after it stopped scrolling. */
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
  /** The changeset on view, by `keyId`; unknown before the first manifest. */
  private key?: string;
  /** The range the views show, if they show one. */
  private shown?: { from: number; to: number; revision: number };
  /** The files the Changes tab was limited to when it was built, if it was. */
  private filtered?: ReadonlySet<string>;
  /** The commit the views show, if they show one. */
  private shownCommit?: number;
  /** What each changeset that is not on view had open, for when it comes back. */
  private parked = new Map<string, SavedView>();
  /** Scroll positions and the current hunk the next builds restore, instead of carrying the old views' over. */
  private restore?: { scroll: Map<TabId, number>; anchor?: SavedView["anchor"] };
  private saving?: ReturnType<typeof setTimeout>;

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
      diagnostics: (revision, path) =>
        this.analysable("target") ? semantic.diagnostics(revision, path) : Promise.resolve([]),
    };
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.onReview((u) => {
      for (const view of this.views.values()) view.applyReview(u);
      for (const [id, text] of this.texts) if (pathOf(id) === u.path) text.applyReview(u.review.hunks);
    });
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
    clearTimeout(this.saving);
    this.savePosition();
    this.unsubscribe();
    this.unsubscribeEvents();
    for (const view of this.views.values()) view.destroy();
    this.views.clear();
    for (const text of this.texts.values()) text.destroy();
    this.texts.clear();
    for (const log of this.logViews.values()) log.destroy();
    this.logViews.clear();
  }

  get files() {
    return this.changeset.files;
  }

  /** The manifest shows a range that does not end at the latest version: nothing can be marked or commented. */
  private get readOnly(): boolean {
    return this.changeset.currentManifest()?.interdiff?.markable === false;
  }

  /** Whether the semantic layer has `side` of what the view shows: not in a commit, whose snapshots are not the changeset's. */
  private analysable(side: Side): boolean {
    if (this.changeset.shownCommit !== undefined) return false;
    const range = this.changeset.currentManifest()?.interdiff;
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
    return {
      tab: { kind: kindOf(id), path: subjectOf(id) },
      tabCount: this.tabs.open.length,
      viewMode: this.activeMode,
      viewModes: modesOf(kindOf(id), path !== undefined && this.isChanged(path)),
      diffView: view !== undefined || text !== undefined,
      byCommit: this.changeset.shownCommit !== undefined,
      hunk: view?.current ?? text?.current ?? -1,
      editor: focusedEditor(document.activeElement),
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

  /** The file of the focused file or text tab; none for the Changes tab. */
  get activePath(): string | undefined {
    return pathOf(this.tabs.active);
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
    const manifest = this.changeset.currentManifest();
    if (root && manifest && !this.texts.has(id)) this.buildText(id, root, manifest);
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

  private listenersFor(id: TabId): Listeners {
    return {
      pinned: id === CHANGES ? (path) => (this.tabs.changesFile = path) : undefined,
      toggleHunk: (revision, path, hunk) =>
        this.editing(
          () => this.changeset.toggleHunk(revision, path, hunk),
          () => undefined,
        ),
      toggleItem: (revision, path, kind) =>
        this.editing(
          () => this.changeset.toggleItem(revision, path, kind),
          () => undefined,
        ),
      toggleFileSeen: (revision, path) =>
        this.editing(
          () => this.changeset.toggleFileSeen(revision, path),
          () => undefined,
        ),
      reconfirmFile: (revision, path) =>
        this.editing(
          () => this.changeset.reconfirmFile(revision, path),
          () => undefined,
        ),
      // Only the changeset has a base diff to show, not a range of versions.
      baseDiff:
        this.changeset.view.type === "changeset"
          ? (revision, path, hunk) => this.changeset.baseDiff(revision, path, hunk)
          : undefined,
      fileMenu: this.fileMenu,
      openSource: (path, line) => void this.openSource(path, line),
      threads: this.threadListeners(),
    };
  }

  private threadListeners(): ThreadListeners {
    return {
      create: (revision, target, body) =>
        this.editing(
          () => threads.create(revision, target, body),
          () => false,
        ),
      commentTarget: (revision, path, at) =>
        this.editing(
          () => this.backend.commentTarget(revision, path, at, this.changeset.view),
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

  /** The threads were read for the revision and the range `manifest` is of. */
  private threadsFit(manifest: Manifest): boolean {
    return threads.revision === manifest.revision && threads.key === rangeKey(manifest.interdiff);
  }

  /** Gives every view the threads, if they are of the view's revision and range. */
  private showThreads() {
    const manifest = this.changeset.currentManifest();
    if (threads.revision === undefined || !manifest || !this.threadsFit(manifest)) return;
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
  private buildText(id: TabId, root: HTMLElement, manifest: Manifest) {
    const path = pathOf(id);
    if (path === undefined) return;
    this.texts.get(id)?.destroy();
    const file = manifest.files.find((f) => f.path === path);
    const text = new TextView(
      root,
      path,
      fileSource(this.backend, manifest.revision, this.changeset.view),
      bindRevision(this.hooks, manifest.revision),
      this.softWrap,
      { revision: manifest.revision, listeners: this.threadListeners() },
      file && { file, actions: this.textActions(file, manifest.revision) },
    );
    this.texts.set(id, text);
    if (this.threadsFit(manifest)) text.setThreads(this.shownThreads(), manifest.revision);
  }

  /**
   * Builds the tab's diff view in its root, replacing `previous`. The scroll position and the
   * current hunk (the nearest one of the same file) carry over.
   */
  private build(id: TabId, manifest: Manifest, previous?: ChangesetView) {
    const root = this.roots.get(id);
    if (!root || kindOf(id) === "log") return;
    const path = pathOf(id);
    const only = this.changeset.baseChanged;
    const files =
      path === undefined
        ? manifest.files.filter((f) => !only || only.has(f.path))
        : manifest.files.filter((f) => f.path === path);
    const restoring = this.restore?.scroll.has(id) ? this.restore : undefined;
    const anchor = restoring ? (id === CHANGES ? restoring.anchor : undefined) : previous?.currentAnchor();
    const top = restoring ? (restoring.scroll.get(id) ?? 0) : root.scrollTop;
    if (restoring) {
      restoring.scroll.delete(id);
      if (restoring.scroll.size === 0) this.restore = undefined;
    }
    previous?.destroy();
    this.views.delete(id);
    if (path !== undefined && !files.length) return; // the file left the changeset
    const view = new ChangesetView(
      root,
      manifest.revision,
      files,
      fileSource(this.backend, manifest.revision, this.changeset.view),
      this.diffModeOf(id),
      this.softWrap,
      this.hooks,
      this.listenersFor(id),
      id === CHANGES, // only the Changes tab collapses unchanged regions
    );
    this.views.set(id, view);
    if (this.threadsFit(manifest)) view.setThreads(this.shownThreads(), manifest.revision);
    root.scrollTop = top;
    view.keepCurrent(anchor);
    view.sync();
    // A file tab shows its one file at once; the Changes tab loads sections as they come near.
    // A stale revision is replaced by the next build.
    if (path !== undefined) view.load(0).catch(() => undefined);
  }

  /** Where the Changes tab was left when the changeset was last open. */
  private savedPosition(): ScrollPosition | undefined {
    const repo = session.repo;
    return repo === undefined || this.key === undefined ? undefined : readPosition(repo, this.key);
  }

  /** Remembers where the Changes tab is scrolled to; a range is not the changeset's own view and is not kept. */
  private savePosition() {
    const repo = session.repo;
    const at = this.changeset.currentManifest()?.interdiff ? undefined : this.views.get(CHANGES)?.position();
    if (repo !== undefined && this.key !== undefined && at) writePosition(repo, this.key, at);
  }

  /** The Changes tab scrolled: the position is saved once the scrolling pauses. */
  private onChangesScroll = () => {
    clearTimeout(this.saving);
    this.saving = setTimeout(() => this.savePosition(), SAVE_AFTER_MS);
  };

  /** A file tab's text pane has its scroll root; returns the function that detaches it. The text is built once the tab shows it. */
  attachText(id: TabId, root: HTMLElement): () => void {
    this.textRoots.set(id, root);
    if (this.tabs.modes.get(id) === "file") this.ensureText(id);
    return () => {
      this.texts.get(id)?.destroy();
      this.texts.delete(id);
      this.textRoots.delete(id);
    };
  }

  /** A tab's pane has its scroll root (for a file tab, that of its diff); returns the function that detaches it. */
  attach(id: TabId, root: HTMLElement): () => void {
    this.roots.set(id, root);
    // The Changes tab is built by `open`, which fetches the manifest first.
    const manifest = this.changeset.currentManifest();
    if (kindOf(id) === "log") this.buildLog(id, root);
    else if (id !== CHANGES && manifest) this.build(id, manifest);
    // Registered before the view is, so it hears the scroll that the view's own listener stops.
    if (id === CHANGES) root.addEventListener("scroll", this.onChangesScroll, { passive: true });
    return () => {
      if (id === CHANGES) root.removeEventListener("scroll", this.onChangesScroll);
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
  private textActions(file: FileChange, revision: number): TextActions {
    return {
      toggleSeen: (hunk) =>
        void this.editing(
          () => this.changeset.toggleHunk(revision, file.path, hunk),
          () => undefined,
        ),
    };
  }

  /** Renders `manifest` in every tab, keeping scroll positions and current hunks (a new revision replaces the views). */
  show(manifest: Manifest) {
    for (const id of this.roots.keys()) this.build(id, manifest, this.views.get(id));
    // A text view is built where the tab shows it or showed it.
    for (const [id, root] of this.textRoots) {
      if (this.texts.has(id) || this.tabs.modes.get(id) === "file") this.buildText(id, root, manifest);
    }
  }

  /**
   * A refetched manifest may be of a newer revision than the views; rebuild them then. A manifest of
   * another changeset first parks the tabs, scroll positions and current hunk of the one that was on
   * view, and brings back those of the new one, if it was on view before.
   */
  update(manifest: Manifest) {
    const key = keyId(manifest.target.changeset);
    const switching = this.key !== undefined && key !== this.key;
    const paths = new Set(manifest.files.map((f) => f.path));
    const firstVisit = switching && !this.parked.has(key);
    if (switching) this.switchTo(key, paths);
    else this.tabs.dropStale((path) => paths.has(path));
    this.key = key;
    const commitChanged = this.commitChanged();
    const rangeChanged = this.rangeChanged(manifest) || commitChanged;
    const filterChanged = this.filtered !== this.changeset.baseChanged;
    this.filtered = this.changeset.baseChanged;
    const view = this.views.get(CHANGES);
    if (!view || (view.revision === manifest.revision && !rangeChanged && !filterChanged)) return;
    // Another range is another set of files: its views start at the top. So does the Changes tab when
    // it shows other files.
    if ((rangeChanged || filterChanged) && !switching) {
      const tabs = this.tabs.open.filter((tab) => hasDiff(tab) && (rangeChanged || tab === CHANGES));
      this.restore = { scroll: new Map(tabs.map((tab) => [tab, 0])) };
    }
    this.show(manifest);
    if (firstVisit || rangeChanged || filterChanged) void this.changes.goToHunk(0);
    // The new views have loaded nothing; a newer revision replaces this load.
    this.changes.loadAll().catch(() => undefined);
  }

  /**
   * Whether `manifest` shows another range than the last one did: no range and a range differ, and
   * so do ranges that start at another version. A live range whose end moved with a new revision
   * is the same range.
   */
  private rangeChanged(manifest: Manifest): boolean {
    const now = manifest.interdiff;
    const was = this.shown;
    this.shown = now && { from: now.from, to: now.to, revision: manifest.revision };
    if (!now || !was) return !now !== !was;
    return now.from !== was.from || (now.to !== was.to && manifest.revision === was.revision);
  }

  /** Whether the views show another commit than they did, or one where there was none or none where there was one. */
  private commitChanged(): boolean {
    const now = this.changeset.shownCommit;
    const was = this.shownCommit;
    this.shownCommit = now;
    return now !== was;
  }

  /** Parks the changeset on view and sets the tabs of `next` and what the builds restore. */
  private switchTo(next: string, paths: ReadonlySet<string>) {
    if (this.key !== undefined) {
      clearTimeout(this.saving);
      this.savePosition();
      const scroll = new Map<TabId, number>();
      for (const [id, root] of this.roots) scroll.set(id, root.scrollTop);
      this.parked.set(this.key, {
        open: [...this.tabs.open],
        active: this.tabs.active,
        modes: new Map(this.tabs.modes),
        scroll,
        anchor: this.views.get(CHANGES)?.currentAnchor(),
      });
    }
    const saved = this.parked.get(next);
    const { open, active, modes } = restoredTabs(saved, (path) => paths.has(path), this.diffMode);
    this.tabs.modes.clear();
    for (const [id, mode] of modes) this.tabs.modes.set(id, mode);
    this.tabs.open = open;
    this.tabs.active = active;
    // Every diff tab builds with its saved position, those not mounted yet when they attach.
    const scroll = new Map(open.filter(hasDiff).map((tab) => [tab, saved?.scroll.get(tab) ?? 0]));
    this.restore = { scroll, anchor: saved?.anchor };
  }

  /** Fetches the manifest and renders it; with `prefetch`, then loads the rest of the Changes tab once its first hunk is painted. */
  async open(prefetch: boolean): Promise<void> {
    this.views.get(CHANGES)?.destroy();
    this.views.delete(CHANGES);
    const manifest = await this.changeset.refresh();
    this.key = keyId(manifest.target.changeset);
    this.rangeChanged(manifest);
    this.filtered = this.changeset.baseChanged;
    this.commitChanged();
    this.show(manifest);
    const view = this.changes;
    const saved = manifest.interdiff ? undefined : this.savedPosition();
    if (!(saved && view.restore(saved))) await view.goToHunk(0);
    await nextPaint();
    if (prefetch) await view.loadAll();
  }

  /** Builds the views anew with the setting, keeping their scroll positions and current hunks. */
  setSoftWrap(wrap: SoftWrap) {
    if (wrap === this.softWrap) return;
    this.softWrap = wrap;
    const manifest = this.changeset.currentManifest();
    if (manifest) this.show(manifest);
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
