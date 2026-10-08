// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it.

import { tick } from "svelte";
import type { CheckLog } from "../../generated/CheckLog";
import type { CheckRun } from "../../generated/CheckRun";
import type { FileChange } from "../../generated/FileChange";
import type { Manifest } from "../../generated/Manifest";
import type { Side } from "../../generated/Side";
import type { TextSpan } from "../../generated/TextSpan";
import { nextPaint } from "../../lib/nextPaint";
import type { Backend } from "../../state/backend";
import type { Changeset } from "../../state/changeset.svelte";
import { semantic } from "../../state/semantic.svelte";
import { session } from "../../state/session.svelte";
import { threads } from "../../state/threads.svelte";
import { rangeKey } from "../../state/timelineRange";
import type { Thread } from "../../generated/Thread";
import { errorText } from "../../lib/errorText";
import { ChangesetView, type Listeners, type ScrollPosition, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import { LogView } from "./editor/LogView";
import type { ViewProbe } from "./editor/viewProbe";
import { bindRevision, focusedEditor } from "./editor/semantic";
import type { Mode } from "../../lib/viewMode";
import { type CommandContext, commands } from "../../state/commands";
import type { HunkActions } from "./editor/hunkPopover";
import { TextView } from "./editor/TextView";
import { type SavedView, restoredTabs } from "./changesetViews";
import { keyId } from "../../state/changesetKey";
import { type ViewActions, viewCommands } from "./reviewCommands";
import { readPosition, writePosition } from "./savedPosition";
import {
  CHANGES,
  type TabId,
  Tabs,
  fileTab,
  kindOf,
  logTab,
  pathOf,
  showsDiff,
  subjectOf,
  textTab,
} from "./Tabs.svelte";

/** How the dev tools measure and test the controller's editors; the controller itself records nothing. */
export interface Probe {
  /** The hooks to give the editors, in place of the controller's own (to time the ones it wraps). */
  hooks(hooks: ViewHooks): ViewHooks;
  /** The listeners to give the views, in place of the controller's own (to see the requests they make). */
  listeners(listeners: Listeners): Listeners;
  /** A view was built for the tab `tab`. */
  view(tab: TabId, view: ViewProbe): void;
  /** A key reached the window and no command took it; true if the probe did. */
  key(event: KeyboardEvent): boolean;
}

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
  readonly tabs = new Tabs();
  /** The view of each attached tab; a file tab has none while its file is not in the changeset. */
  private views = new Map<TabId, ChangesetView>();
  /** The view of each attached text tab. */
  private texts = new Map<TabId, TextView>();
  /** The logs fetched for the log tabs, which are built from them whenever their pane attaches. */
  private logs = new Map<TabId, CheckLog>();
  /** The view of each attached log tab. */
  private logViews = new Map<TabId, LogView>();
  private roots = new Map<TabId, HTMLElement>();
  mode: Mode = "unified";
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
    private probe?: Probe,
  ) {
    // The semantic layer reads the changeset's snapshots: in a range only the new side of the latest version is one.
    const hooks: ViewHooks = {
      hover: (revision, side, path, line, col) =>
        this.analysable(side) ? semantic.hover(revision, side, path, line, col) : Promise.resolve(NO_HOVER),
      definition: (revision, side, path, line, col) => {
        if (this.analysable(side)) void this.goToDefinition(revision, side, path, line, col);
      },
      diagnostics: (revision, path) =>
        this.analysable("target") ? semantic.diagnostics(revision, path) : Promise.resolve([]),
    };
    this.hooks = probe ? probe.hooks(hooks) : hooks;
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.onReview((u) => {
      for (const view of this.views.values()) view.applyReview(u);
      for (const [id, text] of this.texts) if (pathOf(id) === u.path) text.applyReview(u.review.hunks);
    });
    const offThreads = threads.onChange(() => this.showThreads());
    const offCommands = commands.register(...viewCommands(this.viewActions()));
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

  /** The view of the focused tab. */
  private get active(): ChangesetView | undefined {
    return this.views.get(this.tabs.active);
  }

  /** The facts about the review screen that commands read. */
  private facts(): Partial<CommandContext> {
    const id = this.tabs.active;
    const view = this.views.get(id);
    return {
      tab: { kind: kindOf(id), path: subjectOf(id) },
      diffView: view !== undefined,
      byCommit: this.changeset.shownCommit !== undefined,
      hunk: view?.current ?? -1,
      editor: focusedEditor(document.activeElement),
    };
  }

  private viewActions(): ViewActions {
    return {
      step: (delta) => this.stepHunk(delta),
      toggleSeen: () => {
        const view = this.active;
        return !!view && this.toggleBySpace(view);
      },
      reconfirm: () => {
        const done = this.active?.reconfirmCurrent();
        return done !== undefined;
      },
      comment: () => this.active?.commentHere() ?? false,
      jumpToSource: () => {
        const caret = this.tabs.active === CHANGES ? this.active?.caret() : undefined;
        if (!caret) return false;
        void this.openSource(caret.path, caret.line);
        return true;
      },
      stepCommit: (delta) => this.changeset.stepCommit(delta),
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
   * Shows a 0-based `line` of `side`'s text of the file at `path`, marking `span` if given. A
   * changed file opens as its diff tab; an unchanged file (the same text on both sides) as its
   * read-only text tab. A base-side location lands in the base content: the base editor of a
   * side-by-side view, or the target line of the unified view where the deleted lines are drawn.
   * Without a span a base line has no place in the diff views yet, so the diff opens at its start.
   */
  async openLocation(path: string, line: number | undefined, side: Side, span?: TextSpan) {
    // A base path is the file's old path if it was renamed.
    const file = this.files.find((f) => (side === "base" ? (f.oldPath ?? f.path) : f.path) === path);
    if (!file) {
      this.tabs.showText(path);
      await tick();
      const text = this.texts.get(textTab(path));
      if (span) text?.goToSpan(span);
      else if (line !== undefined) text?.goToLine(line);
    } else if (span) {
      this.tabs.show(file.path);
      await tick();
      await this.views.get(fileTab(file.path))?.goToSpan(0, side, span);
    } else await this.openFile(file.path, side === "target" ? line : undefined);
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
    const listeners: Listeners = {
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
      threads: {
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
        confirm: (thread) => threads.confirm(thread),
      },
    };
    return this.probe ? this.probe.listeners(listeners) : listeners;
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
  }

  /** Shows a thread in the Changes tab: its card, or its file or the changeset's top. */
  async goToThread(thread: Thread): Promise<boolean> {
    this.tabs.active = CHANGES;
    return this.views.get(CHANGES)?.goToThread(thread) ?? false;
  }

  /**
   * Builds the tab's view in its root, replacing `previous`. The scroll position and the
   * current hunk (the nearest one of the same file) carry over.
   */
  private build(id: TabId, manifest: Manifest, previous?: ChangesetView) {
    const root = this.roots.get(id);
    if (!root || kindOf(id) === "log") return;
    const path = pathOf(id);
    if (kindOf(id) === "text" && path !== undefined) {
      this.texts.get(id)?.destroy();
      const file = manifest.files.find((f) => f.path === path);
      const text = new TextView(
        root,
        path,
        fileSource(this.backend, manifest.revision, this.changeset.view),
        bindRevision(this.hooks, manifest.revision),
        file && { file, actions: this.hunkActions(file, manifest.revision) },
      );
      this.texts.set(id, text);
      return;
    }
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
      this.mode,
      this.hooks,
      this.listenersFor(id),
      id === CHANGES, // only the Changes tab collapses unchanged regions
      this.probe && ((probe) => this.probe?.view(id, probe)),
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

  /** A tab's pane has its scroll root; returns the function that detaches the tab. */
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
      this.texts.get(id)?.destroy();
      this.texts.delete(id);
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
   * Switches the active file tab between the file's diff and its text, at the line the caret is on.
   * Does nothing in any other tab, or for a file the changeset does not change.
   */
  async toggleFileView() {
    const id = this.tabs.active;
    const path = pathOf(id);
    if (path === undefined || !this.files.some((f) => f.path === path)) return;
    if (kindOf(id) === "file") await this.openSource(path, this.views.get(id)?.caretLine());
    else await this.openFile(path, this.texts.get(id)?.caretLine());
  }

  /** Jump to Source: the file's text tab, at a 0-based target `line` if given. */
  async openSource(path: string, line?: number) {
    this.tabs.showText(path);
    await tick(); // a new tab's pane attaches and builds its view
    if (line !== undefined) this.texts.get(textTab(path))?.goToLine(line);
  }

  /** What a change mark in the file's text tab offers for the file's hunks. */
  private hunkActions(file: FileChange, revision: number): HunkActions {
    return {
      toggleSeen: (hunk) =>
        void this.editing(
          () => this.changeset.toggleHunk(revision, file.path, hunk),
          () => undefined,
        ),
      comment: (hunk) => void this.openHunk(file, hunk, true),
      openInDiff: (hunk) => void this.openHunk(file, hunk, false),
    };
  }

  /** Opens the file's diff tab at its `hunk`-th hunk, with the form for a comment on it if asked. */
  private async openHunk(file: FileChange, hunk: number, comment: boolean) {
    this.tabs.show(file.path);
    await tick(); // a new tab's pane attaches and builds its view
    const view = this.views.get(fileTab(file.path));
    if (!view) return;
    await view.goToHunk(file.nonText.length + hunk);
    if (comment) view.commentOnHunk(hunk);
  }

  /** Opens the file's diff tab, or focuses it if it is open, and goes to a 0-based target `line` if given. */
  async openFile(path: string, line?: number) {
    const id = fileTab(path);
    this.tabs.show(path);
    await tick(); // a new tab's pane attaches and builds its view
    if (line !== undefined) await this.views.get(id)?.goToLine(0, line);
  }

  /** Renders `manifest` in every tab, keeping scroll positions and current hunks (a new revision replaces the views). */
  show(manifest: Manifest) {
    for (const id of this.roots.keys()) this.build(id, manifest, this.views.get(id));
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
    else this.tabs.keepOnly(paths);
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
      const tabs = this.tabs.open.filter((tab) => showsDiff(tab) && (rangeChanged || tab === CHANGES));
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
        scroll,
        anchor: this.views.get(CHANGES)?.currentAnchor(),
      });
    }
    const saved = this.parked.get(next);
    const { open, active } = restoredTabs(saved, (tab) => kindOf(tab) !== "file" || paths.has(pathOf(tab) ?? ""));
    this.tabs.open = open;
    this.tabs.active = active;
    // Every diff tab builds with its saved position, those not mounted yet when they attach.
    const scroll = new Map(open.filter(showsDiff).map((tab) => [tab, saved?.scroll.get(tab) ?? 0]));
    this.restore = { scroll, anchor: saved?.anchor };
  }

  /** Fetches the manifest and renders it; resolves with the time to the Changes tab's first hunk paint. */
  async open(prefetch: boolean): Promise<number> {
    const t0 = performance.now();
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
    const elapsed = performance.now() - t0;
    if (prefetch) await view.loadAll();
    return elapsed;
  }

  setMode(mode: Mode) {
    this.mode = mode;
    for (const view of this.views.values()) view.setMode(mode);
  }

  /** Goes to the hunk `delta` away from the current one in the focused tab. */
  stepHunk(delta: number) {
    const view = this.active;
    if (!view) return;
    // By commit, the next hunk after a commit's last one is the next commit's first.
    const last = this.tabs.active === CHANGES && delta > 0 && view.current >= view.stopCount - 1;
    if (last && this.changeset.stepCommit(1)) return;
    void view.goToHunk(view.current + delta);
  }
}
