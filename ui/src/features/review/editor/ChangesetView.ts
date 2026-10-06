// A scrolling column of file sections: the whole changeset (the Changes tab) or one
// file (a file tab). Section heights are estimated up front from the manifest, so
// scrolling does not reflow, and corrected once the editor is mounted. An editor is
// mounted only for sections near the viewport.

import { EditorView } from "@codemirror/view";
import type { FileChange } from "../../../generated/FileChange";
import type { FileReview } from "../../../generated/FileReview";
import { iconSvg } from "../../../lib/ui/icons";
import type { Backend } from "../../../state/backend";
import type { BarActions } from "./hunkBar";
import { lineDelta } from "./lineDelta";
import { type Handle, type Hooks, type Loaded, type Mode, mountFile } from "./mount";
import { setCurrent, setStates } from "./reviewState";
import { COLLAPSE, type Metrics, sectionLayout } from "./sectionLayout";

/** Time a scroll frame may spend creating editors before the rest wait for the next frame. */
const MOUNT_BUDGET_MS = 6;
/** Scrolling counts as stopped after this long without a scroll event. */
const SETTLE_MS = 120;

const px = (style: CSSStyleDeclaration, name: string) => parseFloat(style.getPropertyValue(name));

export interface Listeners {
  /**
   * Asks the owner of the review state to mark hunks seen or unseen at `revision`.
   * Its new state comes back through `applyReview`.
   */
  setSeen(revision: number, path: string, hunks: number[], seen: boolean): Promise<void>;
  /** Context menu on a file header. */
  fileMenu(path: string, x: number, y: number): void;
  /** Absent in a view of a single file, which already is the file's tab. */
  openFile?(path: string, line: number): void;
}

/** A view's current hunk, as a position that survives a new revision. */
export interface HunkAnchor {
  path: string;
  /** Target line where the hunk starts. */
  line: number;
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
  /** Estimated offset of each hunk bar from the top of the section. */
  hunkTops: number[];
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
  /** While the viewer scrolls only what is in view mounts; the surroundings follow once the scrolling stops. */
  private onScroll = () => {
    this.sync(false);
    clearTimeout(this.settle);
    this.settle = setTimeout(() => this.sync(), SETTLE_MS);
  };
  private settle?: ReturnType<typeof setTimeout>;
  /** A frame request that continues mounting once a frame's budget is spent. */
  private later?: number;

  constructor(
    private root: HTMLElement,
    /** The manifest revision the files are of. */
    readonly revision: number,
    files: FileChange[],
    private backend: Backend,
    public mode: Mode,
    private hooks: Omit<Hooks, keyof BarActions>,
    private listeners: Listeners,
    /** Collapse unchanged regions (the Changes tab); otherwise the full file is shown. */
    private collapse: boolean,
    symbolsOf: (path: string, hunks: number) => (string | undefined)[],
  ) {
    const style = getComputedStyle(root);
    this.m = {
      line: px(style, "--h-line"),
      hunk: px(style, "--h-hunk"),
      header: px(style, "--h-section"),
      collapsed: px(style, "--h-collapsed"),
    };
    let hunks = 0;
    for (const file of files) {
      this.hunkStart.push(hunks);
      hunks += file.hunks.length;
      const el = document.createElement("section");
      el.className = "file";
      const header = document.createElement("header");
      const title = document.createElement("span");
      title.className = "path mono";
      const slash = file.path.lastIndexOf("/") + 1;
      const dir = document.createElement("span");
      dir.className = "dir";
      dir.textContent = file.path.slice(0, slash);
      title.append(dir, file.path.slice(slash));
      title.title = file.path;
      let open: HTMLButtonElement | undefined;
      if (listeners.openFile) {
        const openFile = () => listeners.openFile?.(file.path, file.hunks[0]?.targetStart ?? 0);
        header.addEventListener("dblclick", (e) => {
          if (!(e.target instanceof Element && e.target.closest("button"))) openFile();
        });
        open = document.createElement("button");
        open.className = "flat open";
        open.type = "button";
        open.title = "Open in tab (F4)";
        open.setAttribute("aria-label", "Open in tab");
        open.innerHTML = iconSvg("file", 14, 1.6);
        open.addEventListener("click", openFile);
      }
      const { added, removed } = lineDelta([file]);
      const delta = document.createElement("span");
      delta.className = "delta mono";
      delta.innerHTML = `<span class="add">+${added}</span> <span class="del">−${removed}</span>`;
      const count = document.createElement("span");
      count.className = "count";
      const button = document.createElement("button");
      button.className = "ctl";
      button.textContent = "Seen";
      button.title = "Toggle all hunks of this file seen";
      const index = this.sections.length;
      button.addEventListener("click", () => void this.toggleSeen(index));
      header.append(title, delta, count, ...(open ? [open] : []), button);
      header.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        this.listeners.fileMenu(file.path, e.clientX, e.clientY);
      });
      const body = document.createElement("div");
      body.className = "body";
      el.append(header, body);
      const actions: BarActions = {
        toggleSeen: (h) => void this.toggleHunk(index, h),
        comment: () => {
          // Comments are not wired up yet.
        },
        openFile: listeners.openFile && ((h) => listeners.openFile?.(file.path, file.hunks[h].targetStart)),
      };
      this.sections.push({
        file,
        review: file.review,
        symbols: symbolsOf(file.path, file.hunks.length),
        el,
        body,
        count,
        top: 0,
        height: 0,
        hunkTops: [],
        hooks: { ...hooks, ...actions },
      });
    }
    this.totalHunks = hunks;
    this.root.replaceChildren(...this.sections.map((s) => s.el));
    this.layout();
    this.root.addEventListener("scroll", this.onScroll, { passive: true });
    this.root.addEventListener("click", this.onExpand, true);
  }

  destroy() {
    if (this.later !== undefined) cancelAnimationFrame(this.later);
    this.later = undefined;
    clearTimeout(this.settle);
    this.root.removeEventListener("scroll", this.onScroll);
    this.root.removeEventListener("click", this.onExpand, true);
    for (const fi of this.mounted) this.sections[fi].handle?.destroy();
    this.mounted.clear();
    this.root.replaceChildren();
  }

  get totalHeight(): number {
    const last = this.sections.at(-1);
    return last ? last.top + last.height : 0;
  }

  /** Estimates every section's height; the section's own bottom border is one more pixel. */
  private layout() {
    let top = 0;
    for (const s of this.sections) {
      s.top = top;
      s.height = this.estimate(s);
      s.el.style.height = `${s.height}px`;
      top += s.height;
      this.updateCount(s);
    }
  }

  /** Sets a section's height; sections above the viewport shift what is shown, so the scroll position follows. */
  private resize(s: Section, height: number) {
    const delta = height - s.height;
    if (Math.abs(delta) < 0.5) return;
    const above = s.top + s.height <= this.root.scrollTop;
    s.height = height;
    s.el.style.height = `${height}px`;
    let top = s.top + height;
    for (const next of this.sections.slice(this.sections.indexOf(s) + 1)) {
      next.top = top;
      top += next.height;
    }
    if (above) this.root.scrollTop += delta;
  }

  /**
   * Expanding a collapsed region reveals the lines the bar stood for, which is exact; the
   * editor's own height is not a reliable measure, as CodeMirror estimates what is off screen.
   */
  private onExpand = (e: MouseEvent) => {
    const bar = e.target instanceof Element ? e.target.closest(".cm-collapsedLines") : null;
    const lines = Number(/\d+/.exec(bar?.textContent ?? "")?.[0]);
    const s = this.sections.find((s) => s.el.contains(bar));
    if (s && lines) this.resize(s, s.height + lines * this.m.line - this.m.collapsed);
  };

  /** Lays the section out from the manifest; its own bottom border is one more pixel. */
  private estimate(s: Section): number {
    const layout = sectionLayout(s.file, this.mode, this.m, this.collapse);
    s.hunkTops = layout.hunkTops;
    return layout.height + 1;
  }

  private updateCount(s: Section) {
    const hunks = s.review.hunks;
    s.count.textContent = `${hunks.filter((h) => h === "seen").length}/${hunks.length} hunks seen`;
  }

  /** Drops the editor; the regions it had expanded are collapsed again when it is mounted anew. */
  private unmount(s: Section) {
    s.handle?.destroy();
    s.handle = undefined;
    s.body.textContent = "";
    this.resize(s, this.estimate(s));
  }

  setMode(mode: Mode) {
    const fi = this.indexAt(this.root.scrollTop);
    for (const i of this.mounted) this.unmount(this.sections[i]);
    this.mounted.clear();
    this.mode = mode;
    this.layout();
    if (this.sections.length) this.root.scrollTop = this.sections[fi].top;
    this.sync();
  }

  /** Index of the file with this path, if it is in the view. */
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
      if (r.revision !== this.revision) throw new Error("stale revision");
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

  /** Mounts the editor of a loaded section. */
  private mount(fi: number): Handle {
    const s = this.sections[fi];
    if (s.handle) return s.handle;
    if (!s.loaded) throw new Error(`file ${fi} is not loaded`);
    s.handle = mountFile(
      s.body,
      s.file.path,
      s.loaded,
      { states: s.review.hunks, symbols: s.symbols },
      this.mode,
      s.hooks,
      this.collapse ? COLLAPSE : undefined,
    );
    this.mounted.add(fi);
    if (this.current >= this.hunkStart[fi] && this.current < this.hunkStart[fi] + s.file.hunks.length)
      s.handle.dispatchAll(setCurrent.of(this.current - this.hunkStart[fi]));
    return s.handle;
  }

  /**
   * Unmounts far sections and mounts loaded near ones, the ones in view first. Creating an
   * editor is the expensive step, so a scroll frame mounts only as many as fit a time
   * budget; the rest follow on later frames.
   */
  sync(wide = true) {
    const top = this.root.scrollTop;
    const h = this.root.clientHeight;
    const near = wide
      ? [this.indexAt(Math.max(0, top - h)), this.indexAt(top + 2 * h)]
      : [this.indexAt(top), this.indexAt(top + h)];
    const far = [this.indexAt(Math.max(0, top - 2 * h)), this.indexAt(top + 3 * h)];
    for (const fi of [...this.mounted]) {
      if (fi < far[0] || fi > far[1]) {
        this.unmount(this.sections[fi]);
        this.mounted.delete(fi);
      }
    }
    const middle = top + h / 2;
    const distance = (fi: number) => {
      const s = this.sections[fi];
      return Math.abs(s.top + s.height / 2 - middle);
    };
    const pending: number[] = [];
    for (let fi = near[0]; fi <= near[1]; fi++)
      if (!this.sections[fi].handle && this.sections[fi].loaded) pending.push(fi);
    pending.sort((a, b) => distance(a) - distance(b));
    const started = performance.now();
    for (const [i, fi] of pending.entries()) {
      if (i > 0 && performance.now() - started > MOUNT_BUDGET_MS) {
        this.later ??= requestAnimationFrame(() => {
          this.later = undefined;
          this.sync(wide);
        });
        return;
      }
      this.mount(fi);
    }
  }

  /** The editor of a section, mounted now if it was not; the section must be loaded. */
  private mountedHandle(fi: number): Handle {
    return this.mount(fi);
  }

  async goToFile(fi: number): Promise<Handle> {
    await this.load(fi);
    this.root.scrollTop = this.sections[fi].top;
    const handle = this.mountedHandle(fi);
    this.sync();
    return handle;
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
    const loaded = await this.load(fi);
    const hunk = loaded.hunks[hi];

    if (this.current >= 0) {
      const [pf, ph] = this.locate(this.current);
      if (!(pf === fi && ph === hi)) this.sections[pf].handle?.dispatchAll(setCurrent.of(null));
    }
    this.current = global;

    const handle = this.reveal(fi, hunk.targetStart, hi);
    handle.dispatchAll(setCurrent.of(hi));
  }

  /** Scrolls to a 0-based target line of file `fi` and puts the caret there. */
  async goToLine(fi: number, line: number): Promise<void> {
    await this.load(fi);
    this.reveal(fi, line);
  }

  /** Scrolls near the line by the estimate and mounts the editor; CodeMirror then scrolls to the line exactly. */
  private reveal(fi: number, line: number, hunk?: number): Handle {
    const s = this.sections[fi];
    const offset = hunk === undefined ? 0 : s.hunkTops[hunk] - this.root.clientHeight / 2;
    this.root.scrollTop = Math.max(0, s.top + offset);
    const handle = this.mountedHandle(fi);
    this.sync();
    const doc = handle.view.state.doc;
    const pos = doc.line(Math.min(line + 1, doc.lines)).from;
    handle.view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
    this.sync();
    return handle;
  }

  /** The file and 0-based target line of the focused editor's caret. */
  caret(): { path: string; line: number } | undefined {
    for (const fi of this.mounted) {
      const { view } = this.sections[fi].handle ?? {};
      if (view?.hasFocus)
        return {
          path: this.sections[fi].file.path,
          line: view.state.doc.lineAt(view.state.selection.main.head).number - 1,
        };
    }
    return undefined;
  }

  /** Where the current hunk is, so a view of a newer revision can keep it. */
  currentAnchor(): HunkAnchor | undefined {
    if (this.current < 0) return undefined;
    const [fi, hi] = this.locate(this.current);
    const file = this.sections[fi].file;
    return { path: file.path, line: file.hunks[hi]?.targetStart ?? 0 };
  }

  /** Makes the hunk of the anchor's file nearest to its position the current one, without scrolling. */
  keepCurrent(anchor: HunkAnchor | undefined) {
    const fi = anchor ? this.indexOf(anchor.path) : -1;
    if (!anchor || fi < 0) return;
    const hunks = this.sections[fi].file.hunks;
    let nearest = -1;
    hunks.forEach((h, i) => {
      if (nearest < 0 || Math.abs(h.targetStart - anchor.line) < Math.abs(hunks[nearest].targetStart - anchor.line))
        nearest = i;
    });
    if (nearest >= 0) this.current = this.hunkStart[fi] + nearest;
  }

  /** The latest `toggleSeen`, so callers (the benchmark) can wait for the repaint. */
  pendingToggle: Promise<void> = Promise.resolve();

  /** Asks for the hunks to be marked seen or unseen; the returned state reaches the view through `applyReview` (I5: the view keeps none of its own). */
  private setSeen(fi: number, hunks: number[], value: boolean): Promise<void> {
    const { file } = this.sections[fi];
    return (this.pendingToggle = this.listeners.setSeen(this.revision, file.path, hunks, value));
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
    return this.setSeen(
      fi,
      review.hunks.map((_, i) => i),
      !review.hunks.every((h) => h === "seen"),
    );
  }

  /** Unseen → seen, seen → unseen, seen against an old base → re-confirmed. */
  toggleHunk(fi: number, hi: number): Promise<void> {
    return this.setSeen(fi, [hi], this.sections[fi].review.hunks[hi] !== "seen");
  }

  /** Toggles the current hunk. */
  toggleCurrent(): Promise<void> {
    const [fi, hi] = this.locate(this.current);
    return this.toggleHunk(fi, hi);
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

  /** Largest difference between a mounted section's real height and its estimate; expanded regions are accounted for. */
  maxDrift(): number {
    let drift = 0;
    for (const fi of this.mounted) {
      const s = this.sections[fi];
      drift = Math.max(drift, Math.abs(s.body.scrollHeight + this.m.header + 1 - s.height));
    }
    return drift;
  }

  seenButton(fi: number): HTMLButtonElement {
    const button = this.sections[fi].el.querySelector("button");
    if (!button) throw new Error(`file ${fi} has no Seen button`);
    return button;
  }
}
