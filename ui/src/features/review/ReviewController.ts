// Ties the backend and the views of the open tabs together, and registers the commands that act
// on the active view. Each tab's pane hands its scroll root to `attach`; the controller builds
// the tab's view in it.

import { tick } from "svelte";
import type { Manifest } from "../../generated/Manifest";
import type { Side } from "../../generated/Side";
import type { TextSpan } from "../../generated/TextSpan";
import { nextPaint } from "../../lib/nextPaint";
import type { Backend } from "../../state/backend";
import type { Changeset } from "../../state/changeset.svelte";
import { semantic } from "../../state/semantic.svelte";
import { session } from "../../state/session.svelte";
import { threads } from "../../state/threads.svelte";
import type { Thread } from "../../generated/Thread";
import { ChangesetView, type Listeners, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import type { ViewProbe } from "./editor/viewProbe";
import { bindRevision, focusedEditor } from "./editor/semantic";
import type { Mode } from "../../lib/viewMode";
import { type CommandContext, commands } from "../../state/commands";
import { TextView } from "./editor/TextView";
import { type SavedView, restoredTabs } from "./changesetViews";
import { keyId } from "../../state/changesetKey";
import { type ViewActions, viewCommands } from "./reviewCommands";
import { CHANGES, type TabId, Tabs, fileTab, kindOf, pathOf, textTab } from "./Tabs.svelte";

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
  "files" | "onReview" | "toggleHunk" | "toggleItem" | "toggleFileSeen" | "currentManifest" | "refresh"
>;

export class ReviewController {
  readonly tabs = new Tabs();
  /** The view of each attached tab; a file tab has none while its file is not in the changeset. */
  private views = new Map<TabId, ChangesetView>();
  /** The view of each attached text tab. */
  private texts = new Map<TabId, TextView>();
  private roots = new Map<TabId, HTMLElement>();
  mode: Mode = "unified";
  private hooks: ViewHooks;
  /** The changeset on view, by `keyId`; unknown before the first manifest. */
  private key?: string;
  /** What each changeset that is not on view had open, for when it comes back. */
  private parked = new Map<string, SavedView>();
  /** Scroll positions and the current hunk the next builds restore, instead of carrying the old views' over. */
  private restore?: { scroll: Map<TabId, number>; anchor?: SavedView["anchor"] };

  constructor(
    readonly backend: Backend,
    private changeset: ChangesetSource,
    /** Context menu on a file's header in a view. */
    private fileMenu: (path: string, x: number, y: number) => void,
    private probe?: Probe,
  ) {
    const hooks: ViewHooks = {
      hover: (revision, side, path, line, col) => semantic.hover(revision, side, path, line, col),
      definition: (revision, side, path, line, col) => void this.goToDefinition(revision, side, path, line, col),
      diagnostics: (revision, path) => semantic.diagnostics(revision, path),
    };
    this.hooks = probe ? probe.hooks(hooks) : hooks;
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.onReview((u) => {
      for (const view of this.views.values()) view.applyReview(u);
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
    this.unsubscribe();
    this.unsubscribeEvents();
    for (const view of this.views.values()) view.destroy();
    this.views.clear();
    for (const text of this.texts.values()) text.destroy();
    this.texts.clear();
  }

  get files() {
    return this.changeset.files;
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
      tab: { kind: kindOf(id), path: pathOf(id) },
      diffView: view !== undefined,
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
      comment: () => this.active?.commentHere() ?? false,
      jumpToSource: () => {
        const caret = this.tabs.active === CHANGES ? this.active?.caret() : undefined;
        if (!caret) return false;
        void this.openSource(caret.path, caret.line);
        return true;
      },
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
    if (!this.files.some((f) => f.path === path)) return false;
    this.tabs.active = CHANGES;
    await this.changes.goToFile(this.changes.indexOf(path));
    return true;
  }

  private listenersFor(id: TabId): Listeners {
    const listeners: Listeners = {
      toggleHunk: (revision, path, hunk) => this.changeset.toggleHunk(revision, path, hunk),
      toggleItem: (revision, path, kind) => this.changeset.toggleItem(revision, path, kind),
      toggleFileSeen: (revision, path) => this.changeset.toggleFileSeen(revision, path),
      fileMenu: this.fileMenu,
      openSource: (path, line) => void this.openSource(path, line),
      // A diff tab shows its file's hunks; only the Changes tab offers to open one.
      openDiff: id === CHANGES ? (path, line) => void this.openFile(path, line) : undefined,
      threads: {
        create: (revision, target, body) => threads.create(revision, target, body),
        commentTarget: (revision, path, at) => this.backend.commentTarget(revision, path, at),
        reply: (thread, body) => threads.reply(thread, body),
        setStatus: (thread, status) => threads.setStatus(thread, status),
        remove: (thread) => threads.remove(thread),
        confirm: (thread) => threads.confirm(thread),
      },
    };
    return this.probe ? this.probe.listeners(listeners) : listeners;
  }

  /** Gives every view the threads, if they are of the view's revision. */
  private showThreads() {
    if (threads.revision === undefined) return;
    for (const view of this.views.values()) view.setThreads(threads.items, threads.revision);
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
    if (!root) return;
    const path = pathOf(id);
    if (kindOf(id) === "text" && path !== undefined) {
      this.texts.get(id)?.destroy();
      const text = new TextView(
        root,
        path,
        fileSource(this.backend, manifest.revision),
        bindRevision(this.hooks, manifest.revision),
      );
      this.texts.set(id, text);
      return;
    }
    const files = path === undefined ? manifest.files : manifest.files.filter((f) => f.path === path);
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
      fileSource(this.backend, manifest.revision),
      this.mode,
      this.hooks,
      this.listenersFor(id),
      id === CHANGES, // only the Changes tab collapses unchanged regions
      this.probe && ((probe) => this.probe?.view(id, probe)),
    );
    this.views.set(id, view);
    if (threads.revision === manifest.revision) view.setThreads(threads.items, threads.revision);
    root.scrollTop = top;
    view.keepCurrent(anchor);
    view.sync();
    // A file tab shows its one file at once; the Changes tab loads sections as they come near.
    // A stale revision is replaced by the next build.
    if (path !== undefined) view.load(0).catch(() => undefined);
  }

  /** A tab's pane has its scroll root; returns the function that detaches the tab. */
  attach(id: TabId, root: HTMLElement): () => void {
    this.roots.set(id, root);
    // The Changes tab is built by `open`, which fetches the manifest first.
    const manifest = this.changeset.currentManifest();
    if (id !== CHANGES && manifest) this.build(id, manifest);
    return () => {
      this.views.get(id)?.destroy();
      this.views.delete(id);
      this.texts.get(id)?.destroy();
      this.texts.delete(id);
      this.roots.delete(id);
    };
  }

  /** Jump to Source: the file's text tab, at a 0-based target `line` if given. */
  async openSource(path: string, line?: number) {
    this.tabs.showText(path);
    await tick(); // a new tab's pane attaches and builds its view
    if (line !== undefined) this.texts.get(textTab(path))?.goToLine(line);
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
    const view = this.views.get(CHANGES);
    if (!view || view.revision === manifest.revision) return;
    this.show(manifest);
    if (firstVisit) void this.changes.goToHunk(0);
    // The new views have loaded nothing; a newer revision replaces this load.
    this.changes.loadAll().catch(() => undefined);
  }

  /** Parks the changeset on view and sets the tabs of `next` and what the builds restore. */
  private switchTo(next: string, paths: ReadonlySet<string>) {
    if (this.key !== undefined) {
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
    const scroll = new Map(
      open.filter((tab) => kindOf(tab) !== "text").map((tab) => [tab, saved?.scroll.get(tab) ?? 0]),
    );
    this.restore = { scroll, anchor: saved?.anchor };
  }

  /** Fetches the manifest and renders it; resolves with the time to the Changes tab's first hunk paint. */
  async open(prefetch: boolean): Promise<number> {
    const t0 = performance.now();
    this.views.get(CHANGES)?.destroy();
    this.views.delete(CHANGES);
    const manifest = await this.changeset.refresh();
    this.key = keyId(manifest.target.changeset);
    this.show(manifest);
    const view = this.changes;
    await view.goToHunk(0);
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
    if (view) void view.goToHunk(view.current + delta);
  }
}
