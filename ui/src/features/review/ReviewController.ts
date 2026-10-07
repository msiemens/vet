// Ties the backend, the views of the open tabs and the keybindings together. Each tab's
// pane hands its scroll root to `attach`; the controller builds the tab's view in it.

import { tick } from "svelte";
import type { Manifest } from "../../generated/Manifest";
import type { Side } from "../../generated/Side";
import type { TextSpan } from "../../generated/TextSpan";
import { nextPaint } from "../../lib/nextPaint";
import type { Backend } from "../../state/backend";
import { changeset } from "../../state/changeset.svelte";
import { semantic } from "../../state/semantic.svelte";
import { session } from "../../state/session.svelte";
import { threads } from "../../state/threads.svelte";
import type { Thread } from "../../generated/Thread";
import { ChangesetView, type Listeners, type ViewHooks } from "./editor/ChangesetView";
import { fileSource } from "./editor/fileSource";
import { bindRevision } from "./editor/semantic";
import { KEYS, matches } from "../../lib/keymap";
import type { Mode } from "../../lib/viewMode";
import { TextView } from "./editor/TextView";
import { CHANGES, type TabId, Tabs, fileTab, kindOf, pathOf, textTab } from "./Tabs.svelte";

/** How the dev tools measure and test the controller's editors; the controller itself records nothing. */
export interface Probe {
  /** The hooks to give the editors, in place of the controller's own (to time the ones it wraps). */
  hooks(hooks: ViewHooks): ViewHooks;
  /** A key reached the window and no binding took it; true if the probe did. */
  key(event: KeyboardEvent): boolean;
}

/** Space types in a field or editor and presses a focused control; it marks the current item only when none of those has the focus. */
const takesSpace = (el: Element | null) =>
  !!el?.closest("button, a, input, textarea, select, [contenteditable], [tabindex], [role=menuitem], [role=tab]");

export class ReviewController {
  readonly tabs = new Tabs();
  /** The view of each attached tab; a file tab has none while its file is not in the changeset. */
  private views = new Map<TabId, ChangesetView>();
  /** The view of each attached text tab. */
  private texts = new Map<TabId, TextView>();
  private roots = new Map<TabId, HTMLElement>();
  mode: Mode = "unified";
  private hooks: ViewHooks;

  constructor(
    readonly backend: Backend,
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
    window.addEventListener("keydown", this.onKey, true);
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.onReview((u) => {
      for (const view of this.views.values()) view.applyReview(u);
    });
    const offThreads = threads.onChange(() => this.showThreads());
    this.unsubscribe = () => {
      offReview();
      offThreads();
    };
    // A server installed or reconfigured elsewhere may answer differently now.
    this.unsubscribeEvents = backend.onEvent((event) => {
      if (event.type !== "serversChanged" || event.session !== backend.session) return;
      semantic.invalidate();
      for (const view of this.views.values()) view.refreshDiagnostics();
      for (const text of this.texts.values()) text.refreshDiagnostics();
    });
  }

  private unsubscribe: () => void;
  private unsubscribeEvents: () => void;

  /** Detaches the controller from the window and the store, and tears down the views. */
  destroy() {
    window.removeEventListener("keydown", this.onKey, true);
    this.unsubscribe();
    this.unsubscribeEvents();
    for (const view of this.views.values()) view.destroy();
    this.views.clear();
    for (const text of this.texts.values()) text.destroy();
    this.texts.clear();
  }

  get files() {
    return changeset.files;
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

  private onKey = (e: KeyboardEvent) => {
    const view = this.active;
    if (!view) return;
    const run = (action: () => void) => {
      e.preventDefault();
      action();
    };
    if (matches(KEYS.nextHunk.spec, e)) run(() => void view.goToHunk(view.current + 1));
    else if (matches(KEYS.previousHunk.spec, e)) run(() => void view.goToHunk(view.current - 1));
    else if (matches(KEYS.toggleCurrent.spec, e) && view.current >= 0 && !takesSpace(document.activeElement))
      run(() => void view.toggleCurrent());
    else if (matches(KEYS.commentHere.spec, e) && !document.activeElement?.closest("input, textarea")) {
      // Comment on the selection, else on the current hunk.
      if (view.commentHere()) e.preventDefault();
    } else if (matches(KEYS.jumpToSource.spec, e)) {
      // Jump to Source: the file of the Changes stream's caret, at its line.
      const caret = this.tabs.active === CHANGES ? view.caret() : undefined;
      if (caret) run(() => void this.openSource(caret.path, caret.line));
    } else if (this.probe?.key(e)) e.preventDefault();
  };

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
    return {
      toggleHunk: (revision, path, hunk) => changeset.toggleHunk(revision, path, hunk),
      toggleItem: (revision, path, kind) => changeset.toggleItem(revision, path, kind),
      toggleFileSeen: (revision, path) => changeset.toggleFileSeen(revision, path),
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
      this.texts.set(
        id,
        new TextView(
          root,
          path,
          fileSource(this.backend, manifest.revision),
          bindRevision(this.hooks, manifest.revision),
        ),
      );
      return;
    }
    const files = path === undefined ? manifest.files : manifest.files.filter((f) => f.path === path);
    const anchor = previous?.currentAnchor();
    const top = root.scrollTop;
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
    const manifest = changeset.currentManifest();
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

  /** A refetched manifest may be of a newer revision than the views; rebuild them then. */
  update(manifest: Manifest) {
    const view = this.views.get(CHANGES);
    if (!view || view.revision === manifest.revision) return;
    this.show(manifest);
    // The new views have loaded nothing; a newer revision replaces this load.
    this.changes.loadAll().catch(() => undefined);
  }

  /** Fetches the manifest and renders it; resolves with the time to the Changes tab's first hunk paint. */
  async open(prefetch: boolean): Promise<number> {
    const t0 = performance.now();
    this.views.get(CHANGES)?.destroy();
    this.views.delete(CHANGES);
    this.show(await changeset.refresh());
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
