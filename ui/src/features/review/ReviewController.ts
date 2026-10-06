// Ties the backend, the changeset view and the keybindings together, and
// records the live latencies shown in the dev HUD.

import type { Manifest } from "../../generated/Manifest";
import { nextPaint } from "../../lib/nextPaint";
import type { Backend } from "../../state/backend";
import { changeset } from "../../state/changeset.svelte";
import { ChangesetView } from "./editor/ChangesetView";
import type { Mode } from "./editor/mount";
import { matches } from "./keymap";
import { mockSymbols } from "./mock";

/** Live latencies for the dev HUD. */
export interface Hud {
  keystroke(ms: number): void;
  hover(ms: number): void;
  /** A binding that must never fire on text input fired. */
  collision(total: number): void;
}

/** Bound only to count collisions in the manual input check: AltGr types these keys on a German layout. */
const BAIT = ["Ctrl-Alt-q", "Ctrl-Alt-e", "Ctrl-Alt-7", "Ctrl-Alt-8", "Ctrl-Alt-9", "Ctrl-Alt-0", "Ctrl-Alt-m"];

export class ReviewController {
  view?: ChangesetView;
  mode: Mode = "unified";
  collisions = 0;
  tooltipLatencies: number[] = [];

  constructor(
    readonly backend: Backend,
    readonly root: HTMLElement,
    private hud: Hud,
  ) {
    window.addEventListener("keydown", this.onKey, true);
    this.unsubscribe = changeset.onReview((u) => this.view?.applyReview(u.path, u.review));
  }

  private unsubscribe: () => void;

  /** Detaches the controller from the window and the store, and tears down the view. */
  destroy() {
    window.removeEventListener("keydown", this.onKey, true);
    this.unsubscribe();
    this.view?.destroy();
    this.view = undefined;
  }

  get files() {
    return changeset.files;
  }

  private onKey = (e: KeyboardEvent) => {
    const view = this.view;
    if (!view) return;
    const run = (action: () => void) => (e.preventDefault(), action());
    if (matches("Ctrl-Alt-ArrowDown", e)) run(() => void view.goToHunk(view.current + 1));
    else if (matches("Ctrl-Alt-ArrowUp", e)) run(() => void view.goToHunk(view.current - 1));
    else if (matches("Ctrl-Alt-s", e)) run(() => void view.toggleSeen(view.currentFile()));
    else if (BAIT.some((spec) => matches(spec, e))) run(() => this.hud.collision(++this.collisions));
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

  /** Renders `manifest`, keeping the scroll position (a new revision replaces the view). */
  show(manifest: Manifest) {
    const top = this.root.scrollTop;
    this.view?.destroy();
    this.view = new ChangesetView(
      this.root,
      manifest,
      this.backend,
      this.mode,
      this.hooks,
      { reviewed: (update) => changeset.applyReview(update) },
      mockSymbols,
    );
    this.root.scrollTop = top;
    this.view.sync();
  }

  /** Fetches the manifest and renders it; resolves with the time to the first hunk's paint. */
  async open(prefetch: boolean): Promise<number> {
    const t0 = performance.now();
    this.view?.destroy();
    this.view = undefined;
    this.show(await changeset.refresh());
    const view = this.view!;
    await view.goToHunk(0);
    await nextPaint();
    const elapsed = performance.now() - t0;
    if (prefetch) await view.loadAll();
    return elapsed;
  }

  setMode(mode: Mode) {
    this.mode = mode;
    this.view?.setMode(mode);
  }
}
