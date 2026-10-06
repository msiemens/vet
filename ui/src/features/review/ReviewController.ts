// Ties the backend, the changeset view and the keybindings together, and
// records the live latencies shown in the dev HUD.

import type { Backend } from "../../state/backend";
import type { FileState } from "../../state/changeset.svelte";
import { nextPaint } from "../../lib/nextPaint";
import { ChangesetView, type Listeners as ViewListeners } from "./editor/ChangesetView";
import type { Mode } from "./editor/mount";
import { matches } from "./keymap";
import { mockOldBase, mockSymbols } from "./mock";

/** Live latencies for the dev HUD. */
export interface Hud {
  keystroke(ms: number): void;
  hover(ms: number): void;
  /** A binding that must never fire on text input fired. */
  collision(total: number): void;
}

export interface Listeners extends ViewListeners, Hud {
  opened(name: string, files: FileState[]): void;
}

/** Bound only to count collisions in the manual input check: AltGr types these keys on a German layout. */
const BAIT = ["Ctrl-Alt-q", "Ctrl-Alt-e", "Ctrl-Alt-7", "Ctrl-Alt-8", "Ctrl-Alt-9", "Ctrl-Alt-0", "Ctrl-Alt-m"];

export class ReviewController {
  files: FileState[] = [];
  view?: ChangesetView;
  mode: Mode = "unified";
  collisions = 0;
  tooltipLatencies: number[] = [];

  constructor(
    readonly backend: Backend,
    readonly root: HTMLElement,
    private listeners: Listeners,
  ) {
    window.addEventListener("keydown", (e) => this.onKey(e), true);
  }

  private onKey(e: KeyboardEvent) {
    const view = this.view;
    if (!view) return;
    const run = (action: () => void) => (e.preventDefault(), action());
    if (matches("Ctrl-Alt-ArrowDown", e)) run(() => void view.goToHunk(view.current + 1));
    else if (matches("Ctrl-Alt-ArrowUp", e)) run(() => void view.goToHunk(view.current - 1));
    else if (matches("Ctrl-Alt-s", e)) run(() => void view.toggleSeen(view.currentFile()));
    else if (BAIT.some((spec) => matches(spec, e))) run(() => this.listeners.collision(++this.collisions));
  }

  private keydown = (event: KeyboardEvent) => {
    if (event.key.length !== 1 && event.key !== "Backspace" && event.key !== "Enter") return;
    void nextPaint().then(() => this.listeners.keystroke(performance.now() - event.timeStamp));
  };

  private hooks = {
    hover: async (path: string, line: number, col: number) => {
      const t0 = performance.now();
      const text = await this.backend.hover(path, line, col);
      this.listeners.hover(performance.now() - t0);
      return text;
    },
    hoverShown: (ms: number) => this.tooltipLatencies.push(ms),
    keydown: this.keydown,
  };

  /** Resolves with the time from the open request to the first hunk's paint. */
  async open(prefetch: boolean): Promise<number> {
    const t0 = performance.now();
    this.view?.destroy();
    this.view = undefined;
    const { name, files } = await this.backend.openChangeset();
    this.files = files.map((entry) => ({ entry, seen: [...entry.seen], oldBase: mockOldBase(entry) }));
    this.listeners.opened(name, this.files);
    const view = (this.view = new ChangesetView(this.root, this.files, this.backend, this.mode, this.hooks, this.listeners, mockSymbols));
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
