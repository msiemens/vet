// The whole changeset as one scrolling column of file sections. Section heights
// are known up front (from the manifest), so scrolling never reflows; an editor
// is mounted only for sections near the viewport.

import { EditorView } from "@codemirror/view";
import type { FileChange } from "../../../generated/FileChange";
import type { FileReview } from "../../../generated/FileReview";
import type { Manifest } from "../../../generated/Manifest";
import type { Backend } from "../../../state/backend";
import type { BarActions } from "./hunkBar";
import { type Handle, type Hooks, type Loaded, type Mode, mountFile } from "./mount";
import { setCurrent, setStates } from "./reviewState";

/** Row heights come from the theme tokens, which are the only place they are defined. */
interface Metrics {
  line: number;
  hunk: number;
  header: number;
}

const px = (style: CSSStyleDeclaration, name: string) => parseFloat(style.getPropertyValue(name));

export interface Listeners {
  /** The backend returned a file's new review state; the owner of the state applies it and calls `applyReview`. */
  reviewed(path: string, review: FileReview): void;
}

interface Section {
  file: FileChange;
  review: FileReview;
  symbols: (string | undefined)[];
  el: HTMLElement;
  body: HTMLElement;
  count: HTMLElement;
  top: number;
  height: number;
  hooks: Hooks;
  loaded?: Loaded;
  loading?: Promise<Loaded>;
  handle?: Handle;
}

export class ChangesetView {
  readonly sections: Section[] = [];
  private hunkStart: number[] = [];
  readonly totalHunks: number;
  private mounted = new Set<number>();
  private m: Metrics;
  current = -1;
  private onScroll = () => this.sync();

  constructor(
    private root: HTMLElement,
    readonly manifest: Manifest,
    private backend: Backend,
    public mode: Mode,
    private hooks: Omit<Hooks, keyof BarActions>,
    private listeners: Listeners,
    symbolsOf: (path: string, hunks: number) => (string | undefined)[],
  ) {
    const style = getComputedStyle(root);
    this.m = { line: px(style, "--h-line"), hunk: px(style, "--h-hunk"), header: px(style, "--h-section") };
    let hunks = 0;
    for (const file of manifest.files) {
      this.hunkStart.push(hunks);
      hunks += file.hunks.length;
      const el = document.createElement("section");
      el.className = "file";
      const header = document.createElement("header");
      const title = document.createElement("span");
      title.className = "path";
      title.textContent = file.path;
      const [added, removed] = file.hunks.reduce(([a, r], h) => [a + h.targetLen, r + h.baseLen], [0, 0]);
      const delta = document.createElement("span");
      delta.className = "delta mono";
      delta.innerHTML = `<span class="add">+${added}</span> <span class="del">−${removed}</span>`;
      const count = document.createElement("span");
      count.className = "count";
      const button = document.createElement("button");
      button.textContent = "Seen";
      button.title = "Toggle all hunks of this file seen (Ctrl-Alt-S)";
      const index = this.sections.length;
      button.addEventListener("click", () => void this.toggleSeen(index));
      header.append(title, delta, count, button);
      const body = document.createElement("div");
      body.className = "body";
      el.append(header, body);
      const actions: BarActions = { toggleSeen: (h) => void this.toggleHunk(index, h), comment: () => {} };
      this.sections.push({
        file,
        review: file.review,
        symbols: symbolsOf(file.path, file.hunks.length),
        el,
        body,
        count,
        top: 0,
        height: 0,
        hooks: { ...hooks, ...actions },
      });
    }
    this.totalHunks = hunks;
    this.root.replaceChildren(...this.sections.map((s) => s.el));
    this.layout();
    this.root.addEventListener("scroll", this.onScroll, { passive: true });
  }

  destroy() {
    this.root.removeEventListener("scroll", this.onScroll);
    for (const fi of this.mounted) this.sections[fi].handle?.destroy();
    this.mounted.clear();
    this.root.replaceChildren();
  }

  get totalHeight(): number {
    const last = this.sections[this.sections.length - 1];
    return last ? last.top + last.height : 0;
  }

  private sectionHeight(s: Section): number {
    const { file } = s;
    // Unified shows every deleted line as an extra row; side by side pads the shorter side of each hunk.
    const extra = file.hunks.reduce((rows, h) => rows + (this.mode === "unified" ? h.baseLen : Math.max(0, h.baseLen - h.targetLen)), 0);
    return this.m.header + (file.targetLines + extra) * this.m.line + file.hunks.length * this.m.hunk;
  }

  private layout() {
    let top = 0;
    for (const s of this.sections) {
      s.top = top;
      s.height = this.sectionHeight(s);
      s.el.style.height = `${s.height}px`;
      top += s.height;
      this.updateCount(s);
    }
  }

  private updateCount(s: Section) {
    const hunks = s.review.hunks;
    s.count.textContent = `${hunks.filter((h) => h === "seen").length}/${hunks.length} hunks seen`;
  }

  setMode(mode: Mode) {
    const fi = this.indexAt(this.root.scrollTop);
    for (const i of this.mounted) this.sections[i].handle?.destroy();
    this.mounted.clear();
    for (const s of this.sections) (s.handle = undefined), (s.body.textContent = "");
    this.mode = mode;
    this.layout();
    this.root.scrollTop = this.sections[fi].top;
    this.sync();
  }

  /** Index of the file with this path, if it is in the changeset. */
  indexOf(path: string): number {
    return this.sections.findIndex((s) => s.file.path === path);
  }

  private indexAt(y: number): number {
    let lo = 0;
    let hi = this.sections.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.sections[mid].top <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  load(fi: number): Promise<Loaded> {
    const s = this.sections[fi];
    s.loading ??= this.backend.fileContents(s.file.path).then((r) => {
      if (r.revision !== this.manifest.revision) throw new Error("stale revision");
      s.loaded = { base: r.base ?? "", target: r.target ?? "", hunks: s.file.hunks, marks: r.marks };
      this.sync();
      return s.loaded;
    });
    return s.loading;
  }

  async loadAll(concurrency = 8): Promise<void> {
    let next = 0;
    const worker = async () => {
      while (next < this.sections.length) await this.load(next++);
    };
    await Promise.all(Array.from({ length: concurrency }, worker));
  }

  sync() {
    const top = this.root.scrollTop;
    const h = this.root.clientHeight;
    const near = [this.indexAt(Math.max(0, top - h)), this.indexAt(top + 2 * h)];
    const far = [this.indexAt(Math.max(0, top - 3 * h)), this.indexAt(top + 4 * h)];
    for (const fi of [...this.mounted]) {
      if (fi < far[0] || fi > far[1]) {
        this.sections[fi].handle?.destroy();
        this.sections[fi].handle = undefined;
        this.sections[fi].body.textContent = "";
        this.mounted.delete(fi);
      }
    }
    for (let fi = near[0]; fi <= near[1]; fi++) {
      const s = this.sections[fi];
      if (s.handle || !s.loaded) continue;
      s.handle = mountFile(s.body, s.file.path, s.loaded, { states: s.review.hunks, symbols: s.symbols }, this.mode, s.hooks);
      this.mounted.add(fi);
      if (this.current >= this.hunkStart[fi] && this.current < this.hunkStart[fi] + s.file.hunks.length) s.handle.dispatchAll(setCurrent.of(this.current - this.hunkStart[fi]));
    }
  }

  handleOf(fi: number): Handle | undefined {
    return this.sections[fi].handle;
  }

  async goToFile(fi: number): Promise<Handle> {
    await this.load(fi);
    this.root.scrollTop = this.sections[fi].top;
    this.sync();
    return this.sections[fi].handle!;
  }

  private locate(global: number): [number, number] {
    let fi = this.hunkStart.findIndex((start, i) => start + this.sections[i].file.hunks.length > global);
    if (fi < 0) fi = this.sections.length - 1;
    return [fi, global - this.hunkStart[fi]];
  }

  firstHunkOf(fi: number): number {
    return this.hunkStart[fi];
  }

  async goToHunk(global: number): Promise<void> {
    global = Math.max(0, Math.min(this.totalHunks - 1, global));
    const [fi, hi] = this.locate(global);
    const s = this.sections[fi];
    const loaded = await this.load(fi);
    const hunk = loaded.hunks[hi];

    if (this.current >= 0) {
      const [pf, ph] = this.locate(this.current);
      if (!(pf === fi && ph === hi)) this.sections[pf].handle?.dispatchAll(setCurrent.of(null));
    }
    this.current = global;

    // The row estimate scrolls close enough to mount the editor; CodeMirror then scrolls exactly.
    let row = hunk.targetStart;
    for (const h of loaded.hunks.slice(0, hi)) row += this.mode === "unified" ? h.baseLen : Math.max(0, h.baseLen - h.targetLen);
    this.root.scrollTop = Math.max(0, s.top + this.m.header + row * this.m.line + (hi + 1) * this.m.hunk - this.root.clientHeight / 2);
    this.sync();
    const handle = s.handle!;
    const doc = handle.view.state.doc;
    const pos = doc.line(Math.min(hunk.targetStart + 1, doc.lines)).from;
    handle.dispatchAll(setCurrent.of(hi));
    handle.view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
    this.sync();
  }

  /** The latest `toggleSeen`, so callers (the benchmark) can wait for the repaint. */
  pendingToggle: Promise<void> = Promise.resolve();

  /** Asks the backend to mark the hunks seen or unseen; the returned state reaches the view through `applyReview` (I5: the view keeps none of its own). */
  private setSeen(fi: number, hunks: number[], value: boolean): Promise<void> {
    const { file } = this.sections[fi];
    const request = this.backend.setSeen(this.manifest.revision, file.path, hunks, value).then((review) => this.listeners.reviewed(file.path, review));
    return (this.pendingToggle = request);
  }

  /** Renders a file's review state. */
  applyReview(path: string, review: FileReview) {
    const s = this.sections.find((s) => s.file.path === path);
    if (!s) return;
    s.review = review;
    s.handle?.dispatchAll(setStates.of(review.hunks));
    this.updateCount(s);
  }

  /** Marks all hunks of file `fi` seen, or unseen if they all already are. */
  toggleSeen(fi: number): Promise<void> {
    const { review } = this.sections[fi];
    return this.setSeen(fi, review.hunks.map((_, i) => i), !review.hunks.every((h) => h === "seen"));
  }

  /** Unseen → seen, seen → unseen, seen against an old base → re-confirmed. */
  toggleHunk(fi: number, hi: number): Promise<void> {
    return this.setSeen(fi, [hi], this.sections[fi].review.hunks[hi] !== "seen");
  }

  currentFile(): number {
    return this.current >= 0 ? this.locate(this.current)[0] : this.indexAt(this.root.scrollTop);
  }

  /** Mounted files whose merge chunk count differs from the backend's hunk count (a diagnostic). */
  chunkMismatches(): number {
    let n = 0;
    for (const fi of this.mounted) {
      const s = this.sections[fi];
      if (s.handle && s.handle.chunkCount() !== s.file.hunks.length) n++;
    }
    return n;
  }

  /** Largest difference between a mounted section's real height and its manifest-based height. */
  maxDrift(): number {
    let drift = 0;
    for (const fi of this.mounted) {
      const body = this.sections[fi].body;
      drift = Math.max(drift, Math.abs(body.scrollHeight + this.m.header - this.sections[fi].height));
    }
    return drift;
  }

  seenButton(fi: number): HTMLButtonElement {
    return this.sections[fi].el.querySelector("button")!;
  }
}
