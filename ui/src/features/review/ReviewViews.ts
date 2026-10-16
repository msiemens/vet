// The editor views of the review screen: the diff or log view of each attached tab's pane and the text
// view of each file tab, built in the pane's own scroll root. This also wires review state, diagnostics,
// threads and the find to them, and knows which view and mode a tab is drawn in. The controller builds
// and rebuilds through here; the navigation reads the views.

import { tick, untrack } from "svelte";
import type { CheckLog } from "../../generated/CheckLog";
import type { CheckRun } from "../../generated/CheckRun";
import type { Thread } from "../../generated/Thread";
import { SUBMIT_KEY } from "../../lib/composer";
import { platformName } from "../../lib/platformName";
import type { SoftWrap } from "../../lib/softWrap";
import type { Mode, ViewMode } from "../../lib/viewMode";
import { browse } from "../../state/browse";
import type { Backend } from "../../state/backend";
import type { TabId } from "../../state/commands";
import { commands } from "../../state/commands";
import { contextMenus } from "../../state/contextMenu.svelte";
import { notices } from "../../state/notices.svelte";
import { platform } from "../../state/platform.svelte";
import type { FileLayout, ViewLayout } from "../../state/reviewModel";
import type { ViewSource } from "../../state/viewScope.svelte";
import { type Shown, isCurrent } from "../../state/shown";
import { rangeKey } from "../../state/timelineRange";
import { sameView } from "../../state/view";
import { threads } from "../../state/threads.svelte";
import { threadMenu } from "../comments";
import { ChangesetView, type Listeners, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import { LogView } from "./editor/LogView";
import type { Rendered } from "./editor/rendered";
import { bindRevision } from "./editor/semantic";
import { type TextActions, TextView } from "./editor/TextView";
import { type ThreadActions, buildCard } from "./editor/threadCard";
import type { ThreadListeners } from "./editor/threadLayer";
import type { FindControl } from "./FindControl";
import type { ChangesetSource } from "./changesetSource";
import { CHANGES, commitOf, fileTab, kindOf, logTab, modesOf, pathOf } from "./tabModel";
import type { Tabs } from "./Tabs.svelte";
import type { ViewPersistence } from "./ViewPersistence";
import type { ViewScopes } from "./ViewScopes";

/** Said when a mark or a comment is asked for in a range that does not end at the latest version. */
export const READ_ONLY_NOTE = "This range does not end at the latest version: marks and comments are off";

/** What the views ask of the controller that owns them. */
export interface ViewHost {
  /** Which files the views wrap the long lines of. */
  softWrap: () => SoftWrap;
  /** The changeset on view, which names a new comment's draft. */
  key: () => string | undefined;
  /** The semantic hooks of the tab's view. */
  hooks: (id: TabId) => ViewHooks;
  /** Opens a place a view asked for (Jump to Source), in the tab's own view. */
  openSource: (id: TabId, path: string, line: number) => void;
  /** Context menu on a file's header in a view. */
  fileMenu: (path: string, x: number, y: number) => void;
  /** Remembers the tabs and positions once a pane's scroll pauses. */
  scheduleSave: () => void;
}

export class ReviewViews {
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
  private mode: Mode = "unified";

  private backend: Backend;
  /** The changeset on screen as the views read it: its source, plus the files it is narrowed to and where failures are reported. */
  private changeset: ViewSource & Pick<ChangesetSource, "baseChanged" | "reportFailure">;
  private tabs: Tabs;
  private scopes: ViewScopes;
  private find: FindControl;
  private persistence: ViewPersistence;
  private host: ViewHost;

  constructor(
    backend: Backend,
    changeset: ViewSource & Pick<ChangesetSource, "baseChanged" | "reportFailure">,
    tabs: Tabs,
    scopes: ViewScopes,
    find: FindControl,
    persistence: ViewPersistence,
    host: ViewHost,
  ) {
    this.backend = backend;
    this.changeset = changeset;
    this.tabs = tabs;
    this.scopes = scopes;
    this.find = find;
    this.persistence = persistence;
    this.host = host;
  }

  /** The diff mode chosen last, which a diff tab opens in. */
  get diffMode(): Mode {
    return this.mode;
  }
  set diffMode(mode: Mode) {
    this.mode = mode;
  }

  /** The Changes tab's scroll root. */
  get changesRoot(): HTMLElement {
    const root = this.roots.get(CHANGES);
    if (!root) throw new Error("the Changes tab is not mounted");
    return root;
  }

  /** The Changes tab's view; throws before the first build. */
  get changes(): ChangesetView {
    const view = this.views.get(CHANGES);
    if (!view) throw new Error("no changeset view is mounted");
    return view;
  }

  /** The diff view of a tab, if it is built. */
  diff(id: TabId): ChangesetView | undefined {
    return this.views.get(id);
  }

  /** The text view of a tab, if it is built. */
  text(id: TabId): TextView | undefined {
    return this.texts.get(id);
  }

  /** The scroll root of a tab's diff (or log) view, if its pane is attached. */
  root(id: TabId): HTMLElement | undefined {
    return this.roots.get(id);
  }

  /** The scroll root of a tab's text view, if its pane is attached. */
  textRoot(id: TabId): HTMLElement | undefined {
    return this.textRoots.get(id);
  }

  /** The diff view of the focused tab, unless the tab shows its file's text. */
  activeDiff(): ChangesetView | undefined {
    return this.activeMode === "file" ? undefined : this.views.get(this.tabs.active);
  }

  /** The text of the focused tab, if it shows it and the file is a changed one with hunks to step through. */
  activeText(): TextView | undefined {
    const text = this.activeMode === "file" ? this.texts.get(this.tabs.active) : undefined;
    return text?.hasHunks ? text : undefined;
  }

  /** What the active tab shows; none for a log. */
  get activeMode(): ViewMode | undefined {
    return this.tabs.modes.get(this.tabs.active);
  }

  /** The mounted editors ask for their diagnostics again (those of `path` only, if given). */
  refreshDiagnostics(path?: string) {
    for (const view of this.views.values()) view.refreshDiagnostics(path);
    for (const text of this.texts.values()) text.refreshDiagnostics(path);
  }

  /**
   * Review state was written for `paths`: the views of what is on screen look it up again. The views of
   * another revision or view are about to be replaced and are left alone.
   */
  refreshReview(paths: ReadonlySet<string> | "all") {
    const shown = this.changeset.currentShown();
    if (!shown) return;
    for (const [id, view] of this.views) {
      if (this.scopes.sourceOf(id) === this.changeset && isCurrent(view.shown, shown)) view.refreshReview(paths);
    }
    for (const [id, text] of this.texts) {
      if (this.scopes.sourceOf(id) === this.changeset && text.isFor(shown)) text.refreshReview(paths);
    }
  }

  /** What every view shows of the files' review state, read from what is drawn. */
  rendered(): Rendered {
    return {
      views: Object.fromEntries([...this.views].map(([id, view]) => [id, view.rendered()])),
      texts: Object.fromEntries([...this.texts].map(([id, text]) => [id, text.rendered()])),
    };
  }

  /** Starts the Changes tab loading the rest of its files a few at a time while the reviewer is idle. */
  startTrickle() {
    this.views.get(CHANGES)?.startTrickle();
  }

  /** Gives every view the threads, if they are of the view's revision and range. */
  showThreads() {
    const revision = threads.revision;
    if (revision === undefined) return;
    const fits = (id: TabId) => {
      const layout = this.scopes.sourceOf(id).currentLayout();
      return layout !== undefined && this.threadsFit(layout);
    };
    for (const [id, view] of this.views) if (fits(id)) view.setThreads(this.shownThreads(id), revision);
    for (const [id, text] of this.texts) if (fits(id)) text.setThreads(this.shownThreads(id), revision);
  }

  /**
   * Shows a thread on code: its card in the file's tab, in the mode that tab is in, when the file
   * has a tab open (a tab at a commit shows no threads, so only the file's other tab counts); else in
   * the Changes tab, where it is its card or its file. The changeset's own threads are in no diff.
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

  /**
   * Shows the log of a finished check run in a tab of its own (`shown`: the tab's key and title, which
   * differ per attempt), scrolled to the line where the job failed, else at the top. The log is
   * fetched the first time; a log that cannot be had is reported and opens no tab.
   */
  async openLog(run: CheckRun, shown: { key: string; title: string }) {
    const id = logTab(shown.key);
    let log = this.logs.get(id);
    if (!log) {
      try {
        log = await this.backend.checkLog(run.id);
      } catch (error) {
        notices.failed(`Could not load the log of ${run.name}`, error);
        return;
      }
      this.logs.set(id, log);
    }
    const step = log.failure?.step;
    this.tabs.showLog(shown.key, {
      title: shown.title,
      hint: step ? `${run.name}: the step "${step}" failed` : undefined,
    });
    await tick();
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
    if (!modesOf(kindOf(id), this.scopes.fileChanged(id)).includes(mode)) return;
    const from = this.tabs.modes.get(id);
    if (mode !== "file") this.mode = mode;
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

  /** Draws the tab's diff view in the tab's diff mode. */
  syncDiffMode(id: TabId) {
    const mode = this.tabs.modes.get(id);
    const view = this.views.get(id);
    if (view && mode !== undefined && mode !== "file" && view.mode !== mode) view.setMode(mode);
  }

  /** Builds the tab's text view if the tab has its root and no view yet. */
  ensureText(id: TabId) {
    const root = this.textRoots.get(id);
    if (root && !this.texts.has(id)) this.buildText(id, root);
  }

  /** Renders every attached pane again, keeping scroll positions and current hunks (a new revision replaces the views). */
  show(force = false) {
    for (const id of this.roots.keys()) this.build(id, this.views.get(id), force);
    // A text view is built where the tab shows it or showed it.
    for (const [id, root] of this.textRoots) {
      if (this.texts.has(id) || this.tabs.modes.get(id) === "file") this.buildText(id, root, force);
    }
  }

  /** A tab's pane has its scroll root (for a file tab, that of its diff); returns the function that detaches it. */
  attach(id: TabId, root: HTMLElement): () => void {
    this.roots.set(id, root);
    // Registered before the view is, so it hears the scroll that the view's own listener stops.
    root.addEventListener("scroll", this.host.scheduleSave, { passive: true });
    // The Changes tab is built by `open`, which reads the manifest first.
    if (kindOf(id) === "log") this.buildLog(id, root);
    // Untracked: the pane is not torn down when the state the view reads changes, `show` rebuilds it.
    else if (id !== CHANGES) untrack(() => this.build(id));
    return () => {
      root.removeEventListener("scroll", this.host.scheduleSave);
      // The screen is going away: where the pane was is kept for its rebuild.
      this.persistence.keepPlace(id, "diff", this.views.get(id)?.lines());
      this.views.get(id)?.destroy();
      this.views.delete(id);
      this.logViews.get(id)?.destroy();
      this.logViews.delete(id);
      this.roots.delete(id);
    };
  }

  /** A file tab's text pane has its scroll root; returns the function that detaches it. The text is built once the tab shows it. */
  attachText(id: TabId, root: HTMLElement): () => void {
    this.textRoots.set(id, root);
    // The text view's own scroller does not bubble its scrolls.
    root.addEventListener("scroll", this.host.scheduleSave, { passive: true, capture: true });
    if (this.tabs.modes.get(id) === "file") this.ensureText(id);
    return () => {
      root.removeEventListener("scroll", this.host.scheduleSave, { capture: true });
      this.persistence.keepPlace(id, "text", this.texts.get(id)?.lines());
      this.texts.get(id)?.destroy();
      this.texts.delete(id);
      this.textRoots.delete(id);
    };
  }

  /** Destroys a tab's diff view, so the next build makes one (the Changes tab before a fresh manifest). */
  destroyDiff(id: TabId) {
    this.views.get(id)?.destroy();
    this.views.delete(id);
  }

  /** Detaches every view; the panes' roots and listeners go with the screen. */
  destroy() {
    for (const view of this.views.values()) view.destroy();
    this.views.clear();
    for (const text of this.texts.values()) text.destroy();
    this.texts.clear();
    for (const log of this.logViews.values()) log.destroy();
    this.logViews.clear();
    this.roots.clear();
    this.textRoots.clear();
  }

  /** Builds the tab's diff view in its root, replacing `previous`. The scroll position and the current hunk (the nearest one of the same file) carry over. */
  build(id: TabId, previous?: ChangesetView, force = false) {
    const root = this.roots.get(id);
    if (!root || kindOf(id) === "log") return;
    const source = this.scopes.sourceOf(id);
    const layout = source.currentLayout();
    // A scope's views are rebuilt when its own manifest comes.
    if (!force && previous && source !== this.changeset && layout && isCurrent(previous.shown, layout.shown)) return;
    const path = pathOf(id);
    const only = this.changeset.baseChanged;
    const files =
      path === undefined
        ? (layout?.files ?? []).filter((f) => !only || only.has(f.path))
        : (layout?.files ?? []).filter((f) => f.path === path);
    const restored = this.persistence.takeRestore(id);
    const anchor = restored === undefined ? previous?.currentAnchor() : undefined;
    const top = restored ?? root.scrollTop;
    // The file left the view, or it is not read yet: its place waits for the view that has it again.
    const absent = !layout || (path !== undefined && !files.length);
    if (absent) this.persistence.keepPlace(id, "diff", previous?.lines());
    // A file's tab that is put on another view (a commit, a push, All changes) keeps its lines.
    const place =
      path !== undefined &&
      restored === undefined &&
      layout &&
      previous &&
      !sameView(previous.shown.view, layout.shown.view)
        ? previous.lines()
        : undefined;
    // The files whose editor was mounted are what the reviewer saw: the new view loads them first.
    const seen = previous?.mountedPaths();
    previous?.destroy();
    this.views.delete(id);
    if (absent) return;
    const shown = layout.shown;
    const view = new ChangesetView(
      root,
      shown,
      files,
      source,
      fileSource(this.backend, shown.revision, shown.view),
      this.diffModeOf(id),
      this.host.softWrap(),
      this.host.hooks(id),
      this.listenersFor(id, source, shown),
      id === CHANGES, // only the Changes tab collapses unchanged regions
    );
    this.views.set(id, view);
    if (this.threadsFit(layout)) view.setThreads(this.shownThreads(id), shown.revision);
    view.scrollTo(top);
    view.keepCurrent(anchor);
    if (seen) view.eager(seen);
    view.sync();
    if (this.find.isOpen(id)) this.find.apply(id);
    // A file tab shows its one file at once; the Changes tab loads sections as they come near.
    // A stale revision is replaced by the next build.
    if (path === undefined) return;
    const at = this.persistence.takePending(id, "diff") ?? place;
    (at ? view.showLines(at) : view.load(0)).catch(() => undefined);
  }

  /** Builds a file tab's text view in its root, replacing the one it had. */
  buildText(id: TabId, root: HTMLElement, force = false) {
    const path = pathOf(id);
    if (path === undefined) return;
    const source = this.scopes.sourceOf(id);
    const layout = source.currentLayout();
    const previous = this.texts.get(id);
    // A scope's views are rebuilt when its own manifest comes.
    if (!force && previous && source !== this.changeset && layout && previous.isFor(layout.shown)) return;
    if (!layout) this.persistence.keepPlace(id, "text", previous?.lines());
    const place = layout && previous?.isOfView(layout.shown.view) === false ? previous.lines() : undefined;
    previous?.destroy();
    this.texts.delete(id);
    if (!layout) return;
    const shown = layout.shown;
    const file = layout.files.find((f) => f.path === path);
    const text = new TextView(
      root,
      path,
      fileSource(this.backend, shown.revision, shown.view),
      bindRevision(this.host.hooks(id), shown.revision),
      this.host.softWrap(),
      { shown, listeners: this.threadListeners(source), commentable: commitOf(id) === undefined },
      file && { file, review: source, actions: this.textActions(source, file, shown) },
    );
    this.texts.set(id, text);
    if (this.find.isOpen(id)) this.find.apply(id);
    const at = this.persistence.takePending(id, "text") ?? place;
    if (at) text.showLines(at);
    if (this.threadsFit(layout)) text.setThreads(this.shownThreads(id), shown.revision);
  }

  private buildLog(id: TabId, root: HTMLElement) {
    const log = this.logs.get(id);
    if (log) this.logViews.set(id, new LogView(root, log.text, log.failure?.line));
  }

  /** The diff mode the tab's diff view draws in: the tab's own, or while it shows its text, the one chosen last. */
  private diffModeOf(id: TabId): Mode {
    const mode = this.tabs.modes.get(id);
    return mode === undefined || mode === "file" ? this.mode : mode;
  }

  /** Runs a request that marks or comments in `source`, unless the view on screen is read-only: then it says so and `refuse` answers. */
  private async editing<T>(source: ViewSource, request: () => Promise<T>, refuse: () => T): Promise<T> {
    if (source !== this.changeset || this.changeset.currentLayout()?.interdiff?.markable !== false) return request();
    notices.show(READ_ONLY_NOTE);
    return refuse();
  }

  private listenersFor(id: TabId, source: ViewSource, built: Shown): Listeners {
    return {
      pinned: id === CHANGES ? (path) => (this.tabs.changesFile = path) : undefined,
      toggleHunk: (shown, path, hunk) =>
        this.editing(
          source,
          () => source.toggleHunk(shown, path, hunk),
          () => undefined,
        ),
      toggleFlag: (shown, path, hunk) =>
        this.editing(
          source,
          () => source.toggleFlag(shown, path, hunk),
          () => undefined,
        ),
      toggleItem: (shown, path, kind) =>
        this.editing(
          source,
          () => source.toggleItem(shown, path, kind),
          () => undefined,
        ),
      toggleFileSeen: (shown, path) =>
        this.editing(
          source,
          () => source.toggleFileSeen(shown, path),
          () => undefined,
        ),
      reconfirmFile: (shown, path) =>
        this.editing(
          source,
          () => source.reconfirmFile(shown, path),
          () => undefined,
        ),
      // Only the changeset has a base diff to show, not a range of versions.
      baseDiff: built.view.type === "changeset" ? (shown, path, hunk) => source.baseDiff(shown, path, hunk) : undefined,
      fileMenu: this.host.fileMenu,
      openSource: (path, line) => this.host.openSource(id, path, line),
      loadFailed: (error) => this.changeset.reportFailure(error, built.revision),
      threads: this.threadListeners(source),
      commentable: commitOf(id) === undefined,
    };
  }

  /** What the cards ask of the owner, for threads of the view `source` shows; they go through that view's own admission. */
  private threadListeners(source: ViewSource = this.changeset): ThreadListeners {
    return {
      create: (shown, target, body) =>
        this.editing(
          source,
          () => threads.create(shown, target, body, (sent) => source.admit(sent)),
          () => false,
        ),
      commentTarget: (shown, path, at) =>
        this.editing(
          source,
          () => source.commentTarget(shown, path, at),
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
      openLink: browse,
      submitKey: commands.keyLabel(SUBMIT_KEY),
      draftScope: () => `${this.backend.project}/${this.host.key()}`,
    };
  }

  /** What the file's text tab offers for the file's hunks. */
  private textActions(source: ViewSource, file: FileLayout, shown: Shown): TextActions {
    return {
      toggleSeen: (hunk) =>
        void this.editing(
          source,
          () => source.toggleHunk(shown, file.path, hunk),
          () => undefined,
        ),
      toggleFlag: (hunk) =>
        void this.editing(
          source,
          () => source.toggleFlag(shown, file.path, hunk),
          () => undefined,
        ),
    };
  }

  /** The threads the tab shows: those of the changeset, and none in a commit, where they would sit in the wrong place. */
  private shownThreads(id: TabId): Thread[] {
    return this.scopes.showsCommit(id) ? [] : threads.items;
  }

  /** The threads were read for the revision and the range `layout` is of. */
  private threadsFit(layout: ViewLayout): boolean {
    return threads.revision === layout.shown.revision && threads.key === rangeKey(layout.interdiff);
  }
}
