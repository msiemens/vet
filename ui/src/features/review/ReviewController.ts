// Ties the backend, the views of the open tabs and the keybindings together, and
// records the live latencies shown in the dev HUD. Each tab's pane hands its scroll
// root to `attach`; the controller builds the tab's view in it.

import { tick } from "svelte";
import type { Manifest } from "../../generated/Manifest";
import type { Side } from "../../generated/Side";
import { nextPaint } from "../../lib/nextPaint";
import type { Backend } from "../../state/backend";
import { changeset } from "../../state/changeset.svelte";
import { semantic } from "../../state/semantic.svelte";
import { threads } from "../../state/threads.svelte";
import type { Thread } from "../../generated/Thread";
import { ChangesetView, type Listeners, type ViewHooks } from "./editor/ChangesetView";
import type { Mode } from "./editor/mount";
import { bindRevision } from "./editor/semantic";
import { matches } from "../../lib/keymap";
import { TextView } from "./editor/TextView";
import { CHANGES, type TabId, Tabs, fileTab, kindOf, pathOf, textTab } from "./Tabs.svelte";

/** Live latencies for the dev HUD. */
export interface Hud {
  keystroke(ms: number): void;
  hover(ms: number): void;
  /** A binding that must never fire on text input fired. */
  collision(total: number): void;
}

/** Bound only to count collisions in the manual input check: AltGr types these keys on a German layout. */
const BAIT = ["Ctrl-Alt-q", "Ctrl-Alt-e", "Ctrl-Alt-7", "Ctrl-Alt-8", "Ctrl-Alt-9", "Ctrl-Alt-0", "Ctrl-Alt-m"];

/** Hunk bars show their range only, until the semantic layer names the enclosing symbols. */
const noSymbols = (_path: string, hunks: number): undefined[] => Array.from({ length: hunks }, () => undefined);

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
  collisions = 0;
  tooltipLatencies: number[] = [];

  constructor(
    readonly backend: Backend,
    private hud: Hud,
    /** Context menu on a file's header in a view. */
    private fileMenu: (path: string, x: number, y: number) => void,
  ) {
    window.addEventListener("keydown", this.onKey, true);
    semantic.reset(); // the controller belongs to one repository's screen
    const offReview = changeset.onReview((u) => {
      for (const view of this.views.values()) view.applyReview(u.path, u.review);
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
    if (matches("Ctrl-Alt-ArrowDown", e)) run(() => void view.goToHunk(view.current + 1));
    else if (matches("Ctrl-Alt-ArrowUp", e)) run(() => void view.goToHunk(view.current - 1));
    else if (matches("Space", e) && view.current >= 0 && !takesSpace(document.activeElement))
      run(() => void view.toggleCurrent());
    else if (matches("Ctrl-Alt-c", e) && !document.activeElement?.closest("input, textarea")) {
      // Comment on the selection, else on the current hunk.
      if (view.commentHere()) e.preventDefault();
    } else if (matches("F4", e)) {
      // Jump to Source: the file of the Changes stream's caret, at its line.
      const caret = this.tabs.active === CHANGES ? view.caret() : undefined;
      if (caret) run(() => void this.openSource(caret.path, caret.line));
    } else if (BAIT.some((spec) => matches(spec, e))) run(() => this.hud.collision(++this.collisions));
  };

  private keydown = (event: KeyboardEvent) => {
    if (event.key.length !== 1 && event.key !== "Backspace" && event.key !== "Enter") return;
    void nextPaint().then(() => this.hud.keystroke(performance.now() - event.timeStamp));
  };

  private hooks: ViewHooks = {
    hover: async (revision, side, path, line, col) => {
      const t0 = performance.now();
      const hover = await semantic.hover(revision, side, path, line, col);
      this.hud.hover(performance.now() - t0);
      return hover;
    },
    definition: (revision, side, path, line, col) => void this.goToDefinition(revision, side, path, line, col),
    diagnostics: (revision, path) => semantic.diagnostics(revision, path),
    hoverShown: (ms: number) => this.tooltipLatencies.push(ms),
    keydown: this.keydown,
  };

  /** The file of the focused file or text tab; none for the Changes tab. */
  get activePath(): string | undefined {
    return pathOf(this.tabs.active);
  }

  /**
   * Opens the definition of the symbol at a position of `side`'s text. A location in a changed
   * file opens its diff tab; an unchanged file (the same text on both sides) its read-only text
   * tab. A base-side location lands in the base content: the base editor of a side-by-side view,
   * or the target line of the unified view where the deleted lines are drawn.
   */
  private async goToDefinition(revision: number, side: Side, path: string, line: number, col: number) {
    const found = await semantic.definition(revision, side, path, line, col);
    const location = found?.[0];
    if (!location) return;
    if (location.external) {
      changeset.notice = `Defined outside the repository: ${location.path}`;
      return;
    }
    // A base path is the file's old path if it was renamed.
    const file = this.files.find((f) => (side === "base" ? (f.oldPath ?? f.path) : f.path) === location.path);
    if (file) {
      this.tabs.show(file.path);
      await tick();
      await this.views.get(fileTab(file.path))?.goToSpan(0, side, location.span);
    } else {
      this.tabs.showText(location.path);
      await tick();
      this.texts.get(textTab(location.path))?.goToSpan(location.span);
    }
  }

  private listenersFor(id: TabId): Listeners {
    return {
      setSeen: (revision, path, hunks, seen) => changeset.setSeen(revision, path, hunks, seen),
      acknowledge: (path, kind, acknowledged) => changeset.acknowledge(path, kind, acknowledged),
      fileMenu: this.fileMenu,
      openSource: (path, line) => void this.openSource(path, line),
      // A diff tab shows its file's hunks; only the Changes tab offers to open one.
      openDiff: id === CHANGES ? (path, line) => void this.openFile(path, line) : undefined,
      threads: {
        create: (revision, target, body) => threads.create(revision, target, body),
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
          manifest.revision,
          () => this.backend.fileText(path),
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
      this.backend,
      this.mode,
      this.hooks,
      this.listenersFor(id),
      id === CHANGES, // only the Changes tab collapses unchanged regions
      noSymbols,
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
