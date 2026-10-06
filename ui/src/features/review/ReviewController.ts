// Ties the backend, the views of the open tabs and the keybindings together, and
// records the live latencies shown in the dev HUD. Each tab's pane hands its scroll
// root to `attach`; the controller builds the tab's view in it.

import { tick } from "svelte";
import type { Manifest } from "../../generated/Manifest";
import { nextPaint } from "../../lib/nextPaint";
import type { Backend } from "../../state/backend";
import { changeset } from "../../state/changeset.svelte";
import { ChangesetView, type Listeners } from "./editor/ChangesetView";
import type { Mode } from "./editor/mount";
import { matches } from "../../lib/keymap";
import { TextView } from "./editor/TextView";
import { CHANGES, type TabId, Tabs, fileTab, kindOf, pathOf } from "./Tabs.svelte";

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
    this.unsubscribe = changeset.onReview((u) => {
      for (const view of this.views.values()) view.applyReview(u.path, u.review);
    });
  }

  private unsubscribe: () => void;

  /** Detaches the controller from the window and the store, and tears down the views. */
  destroy() {
    window.removeEventListener("keydown", this.onKey, true);
    this.unsubscribe();
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
    else if (matches("F4", e)) {
      // Jump to Source: the file of the Changes stream's caret, at its line.
      const caret = this.tabs.active === CHANGES ? view.caret() : undefined;
      if (caret) run(() => void this.openFile(caret.path, caret.line));
    } else if (BAIT.some((spec) => matches(spec, e))) run(() => this.hud.collision(++this.collisions));
  };

  private keydown = (event: KeyboardEvent) => {
    if (event.key.length !== 1 && event.key !== "Backspace" && event.key !== "Enter") return;
    void nextPaint().then(() => this.hud.keystroke(performance.now() - event.timeStamp));
  };

  private hooks = {
    hover: async (path: string, line: number, col: number) => {
      const t0 = performance.now();
      const text = await this.backend.hover(path, line, col);
      this.hud.hover(performance.now() - t0);
      return text;
    },
    hoverShown: (ms: number) => this.tooltipLatencies.push(ms),
    keydown: this.keydown,
  };

  private listenersFor(id: TabId): Listeners {
    return {
      setSeen: (revision, path, hunks, seen) => changeset.setSeen(revision, path, hunks, seen),
      fileMenu: this.fileMenu,
      // A file's own tab shows its hunks; only the Changes tab offers to open it.
      openFile: id === CHANGES ? (path, line) => void this.openFile(path, line) : undefined,
    };
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
      this.texts.set(id, new TextView(root, path, manifest.revision, () => this.backend.fileText(path)));
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

  /** Opens a path in its own tab: the diff of a changed file, the text of any other. */
  async openPath(path: string, line?: number) {
    if (changeset.file(path)) return this.openFile(path, line);
    this.tabs.showText(path);
    await tick(); // the pane attaches and builds its view
  }

  /** Opens the file's tab, or focuses it if it is open, and goes to a 0-based target `line` if given. */
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
