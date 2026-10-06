// The whole changeset as one scrolling column of file sections. Section heights
// are known up front (from the manifest), so scrolling never reflows; an editor
// is mounted only for sections near the viewport.

import { EditorView } from "@codemirror/view";
import { type Handle, type Hooks, type Loaded, type Mode, mountFile, setCurrent, setSeen } from "./editor";
import type { FileChange } from "./generated/FileChange";
import type { Manifest } from "./generated/Manifest";
import type { Connection } from "./connection";

export const LINE_H = 18;
const HEADER_H = 28;

export function sectionHeight(entry: FileChange, mode: Mode): number {
  // Unified shows every deleted line as an extra row; side by side pads the shorter side of each hunk.
  const extra = entry.hunks.reduce((rows, h) => rows + (mode === "unified" ? h.baseLen : Math.max(0, h.baseLen - h.targetLen)), 0);
  return HEADER_H + (entry.targetLines + extra) * LINE_H;
}

interface Section {
  entry: FileChange;
  seen: boolean[];
  el: HTMLElement;
  body: HTMLElement;
  count: HTMLElement;
  top: number;
  height: number;
  loaded?: Loaded;
  loading?: Promise<Loaded>;
  handle?: Handle;
}

export class ChangesetView {
  readonly sections: Section[] = [];
  private hunkStart: number[] = [];
  readonly totalHunks: number;
  private mounted = new Set<number>();
  current = -1;
  private onScroll = () => this.sync();

  constructor(
    private root: HTMLElement,
    private manifest: Manifest,
    private conn: Connection,
    public mode: Mode,
    private hooks: Hooks,
  ) {
    let hunks = 0;
    for (const entry of manifest.files) {
      this.hunkStart.push(hunks);
      hunks += entry.hunks.length;
      const el = document.createElement("section");
      el.className = "file";
      const header = document.createElement("header");
      const title = document.createElement("span");
      title.textContent = entry.path;
      const count = document.createElement("span");
      count.className = "count";
      const button = document.createElement("button");
      button.textContent = "Seen";
      button.title = "Toggle all hunks of this file seen (Ctrl-Alt-S)";
      const index = this.sections.length;
      button.addEventListener("click", () => void this.toggleSeen(index));
      header.append(title, count, button);
      const body = document.createElement("div");
      body.className = "body";
      el.append(header, body);
      this.sections.push({ entry, seen: entry.review.hunks.map((s) => s === "seen"), el, body, count, top: 0, height: 0 });
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
    return last.top + last.height;
  }

  private layout() {
    let top = 0;
    for (const s of this.sections) {
      s.top = top;
      s.height = sectionHeight(s.entry, this.mode);
      s.el.style.height = `${s.height}px`;
      top += s.height;
      this.updateCount(s);
    }
  }

  private updateCount(s: Section) {
    s.count.textContent = `${s.seen.filter(Boolean).length}/${s.seen.length} hunks seen`;
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
    const { session } = this.manifest;
    s.loading ??= this.conn.request({ type: "fileContents", session, path: s.entry.path }).then((r) => {
      if (r.type !== "fileContents" || r.revision !== this.manifest.revision) throw new Error("unexpected response");
      s.loaded = { base: r.base ?? "", target: r.target ?? "", hunks: r.hunks, marks: r.marks };
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
      s.handle = mountFile(s.body, s.entry.path, s.loaded, s.seen, this.mode, this.hooks);
      this.mounted.add(fi);
      if (this.current >= this.hunkStart[fi] && this.current < this.hunkStart[fi] + s.entry.hunks.length) s.handle.dispatchAll(setCurrent.of(this.current - this.hunkStart[fi]));
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
    let fi = this.hunkStart.findIndex((start, i) => start + this.sections[i].entry.hunks.length > global);
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
    this.root.scrollTop = Math.max(0, s.top + HEADER_H + row * LINE_H - this.root.clientHeight / 2);
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

  /**
   * Asks the backend to mark all hunks of file `fi` seen, or unseen if they all
   * already are, and renders the state it returns (I5: the shell keeps none of its own).
   */
  toggleSeen(fi: number): Promise<void> {
    const s = this.sections[fi];
    const value = !s.seen.every(Boolean);
    const { session, revision } = this.manifest;
    const request = this.conn.request({ type: "setSeen", session, revision, path: s.entry.path, hunks: s.seen.map((_, i) => i), seen: value }).then((r) => {
      if (r.type !== "review") throw new Error("unexpected response");
      s.seen = r.review.hunks.map((h) => h === "seen");
      s.handle?.dispatchAll(setSeen.of(s.seen));
      this.updateCount(s);
    });
    return (this.pendingToggle = request);
  }

  currentFile(): number {
    return this.current >= 0 ? this.locate(this.current)[0] : this.indexAt(this.root.scrollTop);
  }

  /** Mounted files whose merge chunk count differs from the backend's hunk count (a diagnostic). */
  chunkMismatches(): number {
    let n = 0;
    for (const fi of this.mounted) {
      const s = this.sections[fi];
      if (s.handle && s.handle.chunkCount() !== s.entry.hunks.length) n++;
    }
    return n;
  }

  /** Largest difference between a mounted section's real height and its manifest-based height. */
  maxDrift(): number {
    let drift = 0;
    for (const fi of this.mounted) {
      const body = this.sections[fi].body;
      drift = Math.max(drift, Math.abs(body.scrollHeight + HEADER_H - this.sections[fi].height));
    }
    return drift;
  }

  seenButton(fi: number): HTMLButtonElement {
    return this.sections[fi].el.querySelector("button")!;
  }
}
