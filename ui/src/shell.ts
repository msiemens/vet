// Ties the connection, the changeset view and the keybindings together, and
// records the live latencies shown in the HUD.

import type { Connection } from "./connection";
import type { Mode } from "./editor";
import type { FileEntry } from "./generated/FileEntry";
import { matches } from "./keymap";
import { nextPaint } from "./metrics";
import { ChangesetView } from "./view";

export interface Hud {
  keystroke(ms: number): void;
  hover(ms: number): void;
  /** A binding that must never fire on text input fired. */
  collision(total: number): void;
}

/** Bound only to count collisions in the manual input check: AltGr types these keys on a German layout. */
const BAIT = ["Ctrl-Alt-q", "Ctrl-Alt-e", "Ctrl-Alt-7", "Ctrl-Alt-8", "Ctrl-Alt-9", "Ctrl-Alt-0", "Ctrl-Alt-m"];

export class Shell {
  name = "";
  entries: FileEntry[] = [];
  view?: ChangesetView;
  mode: Mode = "unified";
  collisions = 0;
  tooltipLatencies: number[] = [];

  constructor(
    readonly conn: Connection,
    private root: HTMLElement,
    private hud: Hud,
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
    else if (BAIT.some((spec) => matches(spec, e))) run(() => this.hud.collision(++this.collisions));
  }

  private keydown = (event: KeyboardEvent) => {
    if (event.key.length !== 1 && event.key !== "Backspace" && event.key !== "Enter") return;
    void nextPaint().then(() => this.hud.keystroke(performance.now() - event.timeStamp));
  };

  private hooks = {
    hover: async (path: string, line: number, col: number) => {
      const t0 = performance.now();
      const response = await this.conn.request({ type: "hover", path, line, col });
      this.hud.hover(performance.now() - t0);
      return response.type === "hover" ? response.text : "";
    },
    hoverShown: (ms: number) => this.tooltipLatencies.push(ms),
    keydown: this.keydown,
  };

  /** Resolves with the time from the open request to the first hunk's paint. */
  async open(prefetch: boolean): Promise<number> {
    const t0 = performance.now();
    this.view?.destroy();
    this.view = undefined;
    const response = await this.conn.request({ type: "openChangeset" });
    if (response.type !== "changeset") throw new Error("unexpected response");
    this.name = response.name;
    this.entries = response.files;
    const view = (this.view = new ChangesetView(this.root, this.entries, this.conn, this.mode, this.hooks));
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
