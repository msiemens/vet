// A scrolling column of file sections: the whole changeset (the Changes tab) or one
// file (a file tab). Section heights are estimated up front from the manifest, so
// scrolling does not reflow, and corrected once the editor is mounted. An editor is
// mounted only for sections near the viewport.

import { EditorView, closeHoverTooltips, showTooltip } from "@codemirror/view";
import type { FileChange } from "../../../generated/FileChange";
import type { FileReview } from "../../../generated/FileReview";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { Side } from "../../../generated/Side";
import type { Thread } from "../../../generated/Thread";
import type { TextSpan } from "../../../generated/TextSpan";
import { basename } from "../../../lib/paths";
import type { Mode } from "../../../lib/viewMode";
import type { Backend } from "../../../state/backend";
import { atRevision } from "../../../state/revision";
import { type Card, changeCard } from "./changeCard";
import { iconButton } from "./dom";
import type { BarActions } from "./hunkBar";
import { NO_HIGHLIGHTS, decodeHighlights } from "./highlights";
import { type Handle, type Hooks, type Loaded, mountFile } from "./mount";
import { lineOf, lineStart, selectedLines, targetLine } from "./positions";
import { setCurrent, setStates } from "./reviewState";
import { COLLAPSE, type Metrics, sectionLayout } from "./sectionLayout";
import { type SemanticHooks, type SemanticSource, bindRevision } from "./semantic";
import { type ThreadListeners, type ThreadState, ThreadLayer, threadState } from "./threadLayer";

/** Time a scroll frame may spend creating editors before the rest wait for the next frame. */
const MOUNT_BUDGET_MS = 6;
/** Scrolling counts as stopped after this long without a scroll event. */
const SETTLE_MS = 120;

const px = (style: CSSStyleDeclaration, name: string) => parseFloat(style.getPropertyValue(name));

/** The editor hooks as the owner provides them: the semantic requests still name the view's revision. */
export type ViewHooks = Omit<Hooks, keyof BarActions | keyof SemanticHooks> & SemanticSource;

export interface Listeners {
  /**
   * Asks the owner of the review state to mark hunks seen or unseen at `revision`.
   * Its new state comes back through `applyReview`.
   */
  setSeen(revision: number, path: string, hunks: number[], seen: boolean): Promise<void>;
  /** The same for a non-text item. */
  acknowledge(path: string, kind: NonTextKind, acknowledged: boolean): Promise<void>;
  /** Context menu on a file header. */
  fileMenu(path: string, x: number, y: number): void;
  /** Jump to Source: the file's text at a 0-based target line. */
  openSource(path: string, line: number): void;
  /** The file's own diff tab; absent in a view of a single file, which already is that tab. */
  openDiff?(path: string, line: number): void;
  threads: ThreadListeners;
}

/** A view's current stop, as a position that survives a new revision. */
export interface HunkAnchor {
  path: string;
  /** Target line where the hunk starts. */
  line: number;
  /** Set when the stop is a change card rather than a hunk. */
  card?: NonTextKind;
}

interface Section extends ThreadState {
  file: FileChange;
  review: FileReview;
  el: HTMLElement;
  body: HTMLElement;
  count: HTMLElement;
  /** One per non-text item of the file, above the first hunk. */
  cards: Card[];
  top: number;
  height: number;
  /** Offset of each card from the top of the section. */
  cardTops: number[];
  cardsHeight: number;
  /** Estimated offset of each hunk bar from the top of the section. */
  hunkTops: number[];
  hooks: Hooks;
  loaded?: Loaded;
  loading?: Promise<Loaded>;
  handle?: Handle;
}

export class ChangesetView {
  readonly sections: Section[] = [];
  /**
   * Hunk navigation stops at every change card and every hunk; a file's stops are its cards
   * first, then its hunks. `stopStart[fi]` is the number of stops before file `fi`.
   */
  private stopStart: number[] = [];
  readonly totalHunks: number;
  readonly totalStops: number;
  private mounted = new Set<number>();
  /** Sections whose cards show their image previews. */
  private previewing = new Set<number>();
  private m: Metrics;
  current = -1;
  /** While the viewer scrolls only what is in view mounts; the surroundings follow once the scrolling stops. */
  private onScroll = (e: Event) => {
    // CodeMirror measures each of its editors, synchronously, on a scroll of any ancestor. This
    // listener is registered before any editor mounts, so it runs first and keeps them from it;
    // `measureVisible` asks the visible ones for one measure in this frame's batch instead.
    e.stopImmediatePropagation();
    this.sync(false);
    this.measureVisible();
    clearTimeout(this.settle);
    this.settle = setTimeout(() => this.sync(), SETTLE_MS);
  };
  private settle?: ReturnType<typeof setTimeout>;
  private destroyed = false;
  /** A frame request that continues mounting once a frame's budget is spent. */
  private later?: number;
  private threads: ThreadLayer;

  constructor(
    private root: HTMLElement,
    /** The manifest revision the files are of. */
    readonly revision: number,
    files: FileChange[],
    private backend: Backend,
    public mode: Mode,
    private hooks: ViewHooks,
    private listeners: Listeners,
    /** Collapse unchanged regions (the Changes tab); otherwise the full file is shown. */
    private collapse: boolean,
  ) {
    // Thread cards size themselves to the view's width (`cqw`), not to the width of the longest line.
    root.style.containerType = "inline-size";
    const style = getComputedStyle(root);
    this.m = {
      line: px(style, "--h-line"),
      hunk: px(style, "--h-hunk"),
      header: px(style, "--h-section"),
      collapsed: px(style, "--h-collapsed"),
      gap: px(style, "--h-section-gap"),
      preview: px(style, "--h-preview"),
    };
    this.threads = new ThreadLayer(
      {
        revision,
        sections: this.sections,
        root,
        line: this.m.line,
        estimate: (s, known) => this.estimate(s as Section, known),
        resize: (s, height) => this.resize(s as Section, height),
      },
      listeners.threads,
      collapse,
    );
    let hunks = 0;
    let stops = 0;
    for (const file of files) {
      this.stopStart.push(stops);
      hunks += file.hunks.length;
      stops += file.nonText.length + file.hunks.length;
      const el = document.createElement("section");
      el.className = "file";
      const header = document.createElement("header");
      const title = document.createElement("span");
      title.className = "path mono";
      const name = basename(file.path);
      const dir = document.createElement("span");
      dir.className = "dir";
      dir.textContent = file.path.slice(0, file.path.length - name.length);
      title.append(dir, name);
      title.title = file.path;
      const firstLine = file.hunks[0]?.targetStart ?? 0;
      header.addEventListener("dblclick", (e) => {
        if (!(e.target instanceof Element && e.target.closest("button"))) listeners.openSource(file.path, firstLine);
      });
      const index = this.sections.length;
      const open =
        listeners.openDiff && iconButton("Open diff in tab", "file", () => listeners.openDiff?.(file.path, firstLine));
      const comment = iconButton("Comment on this file", "comments", () =>
        this.threads.commentOnFile(this.sections[index]),
      );
      const { added, removed } = file.delta;
      const delta = document.createElement("span");
      delta.className = "delta mono";
      delta.innerHTML = `<span class="add">+${added}</span> <span class="del">−${removed}</span>`;
      const count = document.createElement("span");
      count.className = "count";
      const button = iconButton(
        "Toggle everything in this file seen",
        "check",
        () => void this.toggleSeen(index),
        "vet-icon-btn seen",
        13,
        2,
      );
      const tools = document.createElement("span");
      tools.className = "tools";
      tools.append(...(open ? [open] : []), comment, button);
      header.append(title, delta, count, tools);
      header.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        this.listeners.fileMenu(file.path, e.clientX, e.clientY);
      });
      const body = document.createElement("div");
      body.className = "body";
      const cards = file.nonText.map((item, ci) =>
        changeCard(file.path, file.oldPath, item, file.review.acknowledged.includes(item.kind), {
          toggle: () => void this.toggleCard(index, ci),
          comment: () => this.threads.commentOnFile(this.sections[index]),
          blob: (side) => this.backend.itemBlob(this.revision, file.path, item.kind, side),
        }),
      );
      const blockEl = document.createElement("div");
      blockEl.className = "vet-thread-block";
      el.append(header, ...cards.map((c) => c.el), blockEl, body);
      el.addEventListener("pointerdown", (e) => this.selectAt(index, e));
      const actions: BarActions = {
        toggleSeen: (h) => void this.toggleHunk(index, h),
        comment: (h) => this.threads.commentOnHunk(this.sections[index], h),
        openSource: (h) => listeners.openSource(file.path, file.hunks[h].targetStart),
      };
      this.sections.push({
        file,
        review: file.review,
        el,
        body,
        count,
        cards,
        top: 0,
        height: 0,
        cardTops: [],
        cardsHeight: 0,
        hunkTops: [],
        ...threadState(blockEl),
        hooks: { ...hooks, ...bindRevision(hooks, revision), ...actions },
      });
    }
    this.totalHunks = hunks;
    this.totalStops = stops;
    this.root.replaceChildren(...(this.threads.lead ? [this.threads.lead] : []), ...this.sections.map((s) => s.el));
    this.layout();
    this.root.addEventListener("scroll", this.onScroll, { passive: true });
    this.root.addEventListener("click", this.onExpand, true);
  }

  destroy() {
    this.destroyed = true;
    if (this.later !== undefined) cancelAnimationFrame(this.later);
    this.later = undefined;
    clearTimeout(this.settle);
    this.root.removeEventListener("scroll", this.onScroll);
    this.root.removeEventListener("click", this.onExpand, true);
    for (const fi of this.mounted) this.sections[fi].handle?.destroy();
    this.mounted.clear();
    for (const fi of this.previewing) this.sections[fi].cards.forEach((c) => c.setPreview(false));
    this.previewing.clear();
    this.threads.destroy();
    this.root.replaceChildren();
  }

  get totalHeight(): number {
    const last = this.sections.at(-1);
    return last ? last.top + last.height : 0;
  }

  /** Estimates every section's height; the section's own bottom border is one more pixel. */
  private layout() {
    let top = this.threads.leadHeight;
    for (const s of this.sections) {
      s.top = top;
      s.height = this.estimate(s);
      s.el.style.height = `${s.height}px`;
      top += s.height;
      this.updateCount(s);
    }
  }

  /**
   * Sets a section's height; sections above the viewport shift what is shown, so the scroll position follows.
   * `scrollTop` is the caller's reading of it: a read after the height writes of a loop forces a layout each time.
   * Returns how far the scroll position moved.
   */
  private resize(s: Section, height: number, scrollTop = this.root.scrollTop): number {
    const delta = height - s.height;
    if (Math.abs(delta) < 0.5) return 0;
    const above = s.top + s.height <= scrollTop;
    s.height = height;
    s.el.style.height = `${height}px`;
    let top = s.top + height;
    for (const next of this.sections.slice(this.sections.indexOf(s) + 1)) {
      next.top = top;
      top += next.height;
    }
    if (above) this.root.scrollTop = scrollTop + delta;
    return above ? delta : 0;
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

  /** Lays the section out from the manifest; its own bottom border is one more pixel. `known`: see `ThreadLayer.count`. */
  private estimate(s: Section, known = new Map<string, number>()): number {
    const space = this.threads.space(s, known);
    const layout = sectionLayout(s.file, this.mode, this.m, this.collapse, space);
    s.hunkTops = layout.hunkTops;
    s.cardTops = layout.cardTops;
    s.cardsHeight = layout.cardsHeight;
    this.threads.count(s, space, layout.drawn, known);
    s.el.style.setProperty("--vet-cards", `${layout.cardsHeight}px`);
    return layout.height + 1;
  }

  private updateCount(s: Section) {
    const { hunks, acknowledged } = s.review;
    const parts: string[] = [];
    if (hunks.length || !s.cards.length)
      parts.push(`${hunks.filter((h) => h === "seen").length}/${hunks.length} hunks seen`);
    if (s.cards.length) parts.push(`${acknowledged.length}/${s.cards.length} other`);
    s.count.textContent = parts.join(" · ");
    s.el.classList.toggle("all-seen", hunks.every((h) => h === "seen") && acknowledged.length === s.cards.length);
  }

  /** Drops the editor; the regions it had expanded are collapsed again when it is mounted anew. */
  private unmount(s: Section, scrollTop = this.root.scrollTop): number {
    s.handle?.destroy();
    s.handle = undefined;
    s.body.textContent = "";
    return this.resize(s, this.estimate(s), scrollTop);
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
    const { file } = s;
    // A file without text lines (binary, content unavailable) has nothing to fetch; the backend refuses it.
    if (!s.loading && !file.hunks.length && !file.baseLines && !file.targetLines)
      s.loading = Promise.resolve(
        (s.loaded = {
          base: "",
          target: "",
          hunks: [],
          changes: [],
          baseHighlights: NO_HIGHLIGHTS,
          targetHighlights: NO_HIGHLIGHTS,
          marks: [],
        }),
      );
    s.loading ??= this.backend.fileContents(file.path).then((reply) => {
      const r = atRevision(this.revision, reply);
      if (!r) throw new Error("stale revision");
      s.loaded = {
        base: r.base ?? "",
        target: r.target ?? "",
        hunks: file.hunks,
        changes: r.changes,
        baseHighlights: decodeHighlights(r.baseHighlights),
        targetHighlights: decodeHighlights(r.targetHighlights),
        marks: r.marks,
      };
      // Mounting waits for the next frame, so a large reply's parse and an editor's creation never share one.
      this.syncNextFrame(true);
      return s.loaded;
    });
    return s.loading;
  }

  /** Loads every section, the ones nearest the viewport first. */
  async loadAll(concurrency = 8): Promise<void> {
    const middle = this.root.scrollTop + this.root.clientHeight / 2;
    const distance = (s: Section) => Math.abs(s.top + s.height / 2 - middle);
    const order = this.sections
      .map((s, i) => i)
      .sort((a, b) => distance(this.sections[a]) - distance(this.sections[b]));
    let next = 0;
    const worker = async () => {
      while (next < order.length) await this.load(order[next++]);
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
      s.review.hunks,
      this.mode,
      s.hooks,
      this.collapse ? COLLAPSE : undefined,
      [this.threads.extension(s)],
    );
    this.threads.mounted(s);
    this.mounted.add(fi);
    const first = this.stopStart[fi] + s.cards.length;
    if (this.current >= first && this.current < first + s.file.hunks.length)
      s.handle.dispatchAll(setCurrent.of(this.current - first));
    this.showDiagnostics(s);
    return s.handle;
  }

  /** Asks for the file's diagnostics (answered from the store's cache after the first time) and shows them once they arrive. */
  private showDiagnostics(s: Section) {
    const { handle, file } = s;
    // A file without target text has nothing a language server could check.
    if (!handle || !file.targetLines) return;
    void s.hooks.diagnostics(file.path).then((list) => {
      if (s.handle === handle && !this.destroyed) handle.showDiagnostics(list);
    });
  }

  /** The servers changed: asks again for the diagnostics of every mounted editor. */
  refreshDiagnostics() {
    for (const fi of this.mounted) this.showDiagnostics(this.sections[fi]);
  }

  /**
   * Unmounts far sections and mounts loaded near ones, the ones in view first. Creating an
   * editor is the expensive step, so a scroll frame mounts only as many as fit a time
   * budget; the rest follow on later frames.
   */
  sync(wide = true) {
    if (this.destroyed) return; // a load that finished after the view was replaced
    let top = this.root.scrollTop;
    const h = this.root.clientHeight;
    const near = wide
      ? [this.indexAt(Math.max(0, top - h)), this.indexAt(top + 2 * h)]
      : [this.indexAt(top), this.indexAt(top + h)];
    const far = [this.indexAt(Math.max(0, top - 2 * h)), this.indexAt(top + 3 * h)];
    for (const fi of [...this.mounted]) {
      if (fi < far[0] || fi > far[1]) {
        top += this.unmount(this.sections[fi], top);
        this.mounted.delete(fi);
      }
    }
    for (let fi = near[0]; fi <= near[1]; fi++) this.setPreviews(fi, true);
    for (const fi of [...this.previewing]) if (fi < far[0] || fi > far[1]) this.setPreviews(fi, false);
    const middle = top + h / 2;
    const distance = (fi: number) => {
      const s = this.sections[fi];
      return Math.abs(s.top + s.height / 2 - middle);
    };
    const pending: number[] = [];
    for (let fi = near[0]; fi <= near[1]; fi++) {
      const { handle, loaded, file } = this.sections[fi];
      // A file without text lines has no editor to show.
      if (!handle && loaded && (file.hunks.length || file.baseLines || file.targetLines)) pending.push(fi);
    }
    pending.sort((a, b) => distance(a) - distance(b));
    const started = performance.now();
    for (const [i, fi] of pending.entries()) {
      if (i > 0 && performance.now() - started > MOUNT_BUDGET_MS) {
        this.syncNextFrame(wide);
        return;
      }
      this.mount(fi);
    }
  }

  private syncNextFrame(wide: boolean) {
    this.later ??= requestAnimationFrame(() => {
      this.later = undefined;
      this.sync(wide);
    });
  }

  /**
   * Requests a measure of the editors in view (CodeMirror runs them in the next frame, once per
   * editor however many scroll events came before), which keeps a tall editor's visible lines rendered while the
   * root scrolls. A hover tooltip is closed rather than followed: the scroll moved its anchor.
   */
  private measureVisible() {
    const top = this.root.scrollTop;
    const bottom = top + this.root.clientHeight;
    for (const fi of this.mounted) {
      const { top: start, height, handle } = this.sections[fi];
      if (!handle || start >= bottom || start + height <= top) continue;
      for (const view of handle.views) {
        if (view.state.facet(showTooltip).some(Boolean)) view.dispatch({ effects: closeHoverTooltips });
        view.requestMeasure();
      }
    }
  }

  /** Image previews are fetched for sections near the viewport and dropped when far away. */
  private setPreviews(fi: number, on: boolean) {
    if (on === this.previewing.has(fi)) return;
    const previews = this.sections[fi].cards.filter((c) => c.hasPreview);
    if (!previews.length) return;
    if (on) this.previewing.add(fi);
    else this.previewing.delete(fi);
    for (const card of previews) card.setPreview(on);
  }

  /** Scrolls the file's header to the top and makes its first card or hunk the current stop. */
  async goToFile(fi: number): Promise<Handle> {
    await this.load(fi);
    this.root.scrollTop = this.sections[fi].top;
    const handle = this.mount(fi);
    this.sync();
    const { cards, file } = this.sections[fi];
    if (cards.length + file.hunks.length) this.select(this.stopStart[fi]);
    return handle;
  }

  /** Makes `stop` the current one without scrolling. */
  private select(stop: number) {
    if (this.current === stop) return;
    if (this.current >= 0) this.markCurrent(this.current, false);
    this.current = stop;
    this.markCurrent(stop, true);
  }

  /**
   * A press inside file `fi` selects what it lands on: a change card, the hunk at or above the
   * pressed line (either side of a split view), or the file's first stop on its header.
   */
  private selectAt(fi: number, e: PointerEvent) {
    const s = this.sections[fi];
    if (!(e.target instanceof Node) || s.cards.length + s.file.hunks.length === 0) return;
    const ci = s.cards.findIndex((c) => c.el.contains(e.target as Node));
    if (ci >= 0) {
      this.select(this.stopStart[fi] + ci);
      return;
    }
    const views = s.handle?.views ?? [];
    const view = views.find((v) => v.dom.contains(e.target as Node));
    if (!view || !s.file.hunks.length) {
      this.select(this.stopStart[fi]);
      return;
    }
    const line = lineOf(view.state.doc, view.posAtCoords({ x: e.clientX, y: e.clientY }, false));
    const base = view !== s.handle?.view;
    let hunk = 0;
    s.file.hunks.forEach((h, i) => {
      if ((base ? h.baseStart : h.targetStart) <= line) hunk = i;
    });
    this.select(this.stopStart[fi] + s.cards.length + hunk);
  }

  /** The file of a stop, and the stop's index among that file's stops: below its card count it is a card, else a hunk. */
  private locate(stop: number): [number, number] {
    const end = (i: number) => this.stopStart[i] + this.sections[i].cards.length + this.sections[i].file.hunks.length;
    let [lo, hi] = [0, this.sections.length - 1];
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (end(mid) > stop) hi = mid;
      else lo = mid + 1;
    }
    return [lo, stop - this.stopStart[lo]];
  }

  /** The stop of file `fi`'s first hunk. */
  firstHunkOf(fi: number): number {
    return this.stopStart[fi] + this.sections[fi].cards.length;
  }

  /** Shows a stop as current or not: a card outlines itself, a hunk is the editor's. */
  private markCurrent(stop: number, on: boolean) {
    const [fi, si] = this.locate(stop);
    const s = this.sections[fi];
    if (si < s.cards.length) s.cards[si].setCurrent(on);
    else s.handle?.dispatchAll(setCurrent.of(on ? si - s.cards.length : null));
  }

  /** Goes to the stop with this index over all files' cards and hunks (a card, then the file's hunks). */
  async goToHunk(stop: number): Promise<void> {
    if (!this.totalStops) return;
    stop = Math.max(0, Math.min(this.totalStops - 1, stop));
    const [fi, si] = this.locate(stop);
    const s = this.sections[fi];
    const cards = s.cards.length;
    if (si < cards) {
      if (this.current >= 0 && this.current !== stop) this.markCurrent(this.current, false);
      this.current = stop;
      // A card's position is exact, whether or not the section's editor is mounted.
      this.root.scrollTop = Math.max(0, s.top + s.cardTops[si] - this.root.clientHeight / 2);
      this.sync();
      this.markCurrent(stop, true);
      return;
    }
    const hi = si - cards;
    const loaded = await this.load(fi);
    const hunk = loaded.hunks[hi];

    if (this.current >= 0 && this.current !== stop) this.markCurrent(this.current, false);
    this.current = stop;

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
    const handle = this.mount(fi);
    this.sync();
    const pos = lineStart(handle.view.state.doc, line);
    handle.view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
    this.sync();
    return handle;
  }

  /**
   * Scrolls to a span of file `fi` as `side`'s text has it and marks it. A unified view has no
   * base editor: a base line is shown at the target line the diff puts it at (a deleted line, at
   * its hunk), and only the line is revealed.
   */
  async goToSpan(fi: number, side: Side, span: TextSpan): Promise<void> {
    await this.load(fi);
    const line = targetLine(this.sections[fi].file.hunks, side, span.start.line);
    this.reveal(fi, line).revealSpan(side, span);
  }

  /** The file and 0-based target line of the focused editor's caret. */
  caret(): { path: string; line: number } | undefined {
    for (const fi of this.mounted) {
      const { view } = this.sections[fi].handle ?? {};
      if (view?.hasFocus)
        return {
          path: this.sections[fi].file.path,
          line: lineOf(view.state.doc, view.state.selection.main.head),
        };
    }
    return undefined;
  }

  /** Where the current stop is, so a view of a newer revision can keep it. */
  currentAnchor(): HunkAnchor | undefined {
    if (this.current < 0 || !this.totalStops) return undefined;
    const [fi, si] = this.locate(this.current);
    const { file, cards } = this.sections[fi];
    if (si < cards.length) return { path: file.path, line: 0, card: file.nonText[si].kind };
    return { path: file.path, line: file.hunks[si - cards.length]?.targetStart ?? 0 };
  }

  /**
   * Makes the stop of the anchor's file nearest to it the current one, without scrolling: the same
   * card, else the hunk nearest to the anchor's line, else the file's first stop.
   */
  keepCurrent(anchor: HunkAnchor | undefined) {
    const fi = anchor ? this.indexOf(anchor.path) : -1;
    if (!anchor || fi < 0) return;
    const { file, cards } = this.sections[fi];
    const card = anchor.card ? file.nonText.findIndex((i) => i.kind === anchor.card) : -1;
    if (card >= 0) {
      this.current = this.stopStart[fi] + card;
      return;
    }
    const hunks = file.hunks;
    let nearest = -1;
    hunks.forEach((h, i) => {
      if (nearest < 0 || Math.abs(h.targetStart - anchor.line) < Math.abs(hunks[nearest].targetStart - anchor.line))
        nearest = i;
    });
    if (nearest >= 0) this.current = this.stopStart[fi] + cards.length + nearest;
    else if (cards.length) this.current = this.stopStart[fi];
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
    s.cards.forEach((card, i) => card.setAcknowledged(review.acknowledged.includes(s.file.nonText[i].kind)));
    this.updateCount(s);
  }

  /** Marks everything in file `fi` (hunks and change cards) seen, or unseen if it all already is. */
  toggleSeen(fi: number): Promise<void> {
    const { review, file } = this.sections[fi];
    const value = !(
      review.hunks.every((h) => h === "seen") && file.nonText.every((i) => review.acknowledged.includes(i.kind))
    );
    const jobs: Promise<void>[] = [];
    if (review.hunks.length)
      jobs.push(
        this.setSeen(
          fi,
          review.hunks.map((_, i) => i),
          value,
        ),
      );
    for (const item of file.nonText)
      if (review.acknowledged.includes(item.kind) !== value)
        jobs.push(this.listeners.acknowledge(file.path, item.kind, value));
    return (this.pendingToggle = Promise.all(jobs).then(() => undefined));
  }

  /** Unseen → seen, seen → unseen, seen against an old base → re-confirmed. */
  toggleHunk(fi: number, hi: number): Promise<void> {
    return this.setSeen(fi, [hi], this.sections[fi].review.hunks[hi] !== "seen");
  }

  /** Acknowledges the file's `ci`-th change card, or takes it back. */
  toggleCard(fi: number, ci: number): Promise<void> {
    const { file, review } = this.sections[fi];
    const { kind } = file.nonText[ci];
    return (this.pendingToggle = this.listeners.acknowledge(file.path, kind, !review.acknowledged.includes(kind)));
  }

  /** Toggles the current stop: a hunk is marked seen or unseen, a card acknowledged or not. */
  toggleCurrent(): Promise<void> {
    const [fi, si] = this.locate(this.current);
    const cards = this.sections[fi].cards.length;
    return si < cards ? this.toggleCard(fi, si) : this.toggleHunk(fi, si - cards);
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
      drift = Math.max(
        drift,
        Math.abs(s.body.scrollHeight + this.m.header + s.cardsHeight + s.block + this.m.gap + 1 - s.height),
      );
    }
    return drift;
  }

  /**
   * Shows the threads of this view's revision: a card below the last line of each range thread,
   * the file's own below its change cards, the changeset's above the first file.
   */
  setThreads(threads: Thread[], revision: number) {
    this.threads.setThreads(threads, revision);
  }

  /**
   * Opens the form for a comment on the focused editor's selection (its caret line without one),
   * else on the current hunk. Answers whether there was somewhere to comment.
   */
  commentHere(): boolean {
    for (const fi of this.mounted) {
      const handle = this.sections[fi].handle;
      const view = handle?.views.find((v) => v.hasFocus);
      if (!handle || !view) continue;
      const { main } = view.state.selection;
      const { doc } = view.state;
      const [first, last] = [lineOf(doc, main.from), lineOf(doc, main.to)];
      const { start, len } = selectedLines(first, last, !main.empty && main.to === lineStart(doc, last));
      this.threads.commentOnLines(this.sections[fi], view === handle.view ? "target" : "base", start, len);
      return true;
    }
    if (this.current < 0 || !this.totalStops) return false;
    const [fi, si] = this.locate(this.current);
    const s = this.sections[fi];
    if (si < s.cards.length) this.threads.commentOnFile(s);
    else this.threads.commentOnHunk(s, si - s.cards.length);
    return true;
  }

  /** Scrolls to where a thread is shown; answers whether it is in this view. */
  async goToThread(thread: Thread): Promise<boolean> {
    if (thread.kind === "changeset") {
      if (!this.threads.lead) return false;
      this.root.scrollTop = 0;
      return true;
    }
    const fi = thread.path === null ? -1 : this.indexOf(thread.path);
    if (fi < 0) return false;
    const card = this.sections[fi].placement.inline.find((c) => c.thread.id === thread.id);
    if (card) await this.goToLine(fi, card.line);
    else await this.goToFile(fi);
    return true;
  }

  seenButton(fi: number): HTMLButtonElement {
    const button = this.sections[fi].el.querySelector<HTMLButtonElement>("header .seen");
    if (!button) throw new Error(`file ${fi} has no Seen button`);
    return button;
  }
}
