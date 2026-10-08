// A scrolling column of file sections: the whole changeset (the Changes tab) or one
// file (a file tab). Section heights are estimated up front from the manifest, so
// scrolling does not reflow, and corrected once the editor is mounted. An editor is
// mounted only for sections near the viewport.

import { EditorView } from "@codemirror/view";
import type { BaseDiff } from "../../../generated/BaseDiff";
import type { FileChange } from "../../../generated/FileChange";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { ReviewUpdate } from "../../../generated/ReviewUpdate";
import type { Side } from "../../../generated/Side";
import type { Thread } from "../../../generated/Thread";
import type { TextSpan } from "../../../generated/TextSpan";
import type { Mode } from "../../../lib/viewMode";
import { changeCard } from "./changeCard";
import { takesKeys } from "./dom";
import { fileHeader, renderProgress } from "./fileHeader";
import type { FileSource } from "./fileSource";
import type { BarActions } from "./hunkBar";
import { setColumns } from "./longLines";
import { type Handle, type Hooks, type Loaded, mountFile } from "./mount";
import { lineOf, lineStart } from "./positions";
import { type ReviewRequests, ReviewActions, renderReview } from "./reviewActions";
import { setBaseDiffs, setCurrent } from "./reviewState";
import { StickyHunk } from "./stickyHunk";
import { ScrollMarks, sectionMarks } from "./scrollMarks";
import { Stops, hunkAt } from "./stops";
import type { Section } from "./section";
import { SectionHeights, charWidth, readFont, readMetrics } from "./sectionHeights";
import { COLLAPSE, type WrappedSides, baseDiffHeight } from "./sectionLayout";
import { type SemanticHooks, type SemanticSource, bindRevision } from "./semantic";
import { wrapsByDefault } from "./softWrap";
import { type ThreadListeners, ThreadLayer, threadState } from "./threadLayer";
import { type ViewProbe, viewProbe } from "./viewProbe";
import { Windowing } from "./windowing";
import { WrapRows, rowColumns } from "./wrapRows";

/** The editor hooks as the owner provides them: the semantic requests still name the view's revision. */
export type ViewHooks = Omit<Hooks, keyof BarActions | keyof SemanticHooks | "openLine"> & SemanticSource;

export interface Listeners extends ReviewRequests {
  /** Context menu on a file header. */
  fileMenu(path: string, x: number, y: number): void;
  /** Jump to Source: the file's text at a 0-based target line. */
  openSource(path: string, line: number): void;
  /** The file whose header is pinned at the top of the view changed; none above the first file. */
  pinned?(path: string | undefined): void;
  threads: ThreadListeners;
}

/** Where a view is scrolled to: below the top of a file's section. */
export interface ScrollPosition {
  path: string;
  /** In px from the section's top; negative above the first section. */
  offset: number;
}

/** A view's current stop, as a position that survives a new revision. */
export interface HunkAnchor {
  path: string;
  /** Target line where the hunk starts. */
  line: number;
  /** Set when the stop is a change card rather than a hunk. */
  card?: NonTextKind;
}

export class ChangesetView {
  private sections: Section[] = [];
  /** Hunk navigation stops at every change card and every hunk. */
  private stops: Stops;
  readonly totalHunks: number;
  private heights: SectionHeights;
  private windowing: Windowing;
  private actions: ReviewActions;
  current = -1;
  private destroyed = false;
  private threads: ThreadLayer;
  private pin: StickyHunk;
  /** The editor's font, the width of a character of it and the view's width: what lines wrap at. */
  private font: string;
  private char: number;
  private width: number;
  private widths: ResizeObserver;
  /** Wrapping sections whose editor measured another height than the section has. */
  private measured = new Set<Section>();
  private measuring?: number;
  private marks?: ScrollMarks;

  constructor(
    private root: HTMLElement,
    /** The manifest revision the files are of. */
    readonly revision: number,
    files: FileChange[],
    private source: FileSource,
    mode: Mode,
    private hooks: ViewHooks,
    private listeners: Listeners,
    /** Collapse unchanged regions (the Changes tab); otherwise the full file is shown. */
    private collapse: boolean,
    /** Hands the dev tools what they measure; production owners leave it out. */
    inspect?: (probe: ViewProbe) => void,
  ) {
    // Thread cards size themselves to the view's width (`cqw`), not to the width of the longest line.
    root.style.containerType = "inline-size";
    const metrics = readMetrics(root);
    this.font = readFont(root);
    this.char = charWidth(this.font);
    // A font that is not loaded yet measures as its fallback; the lines wrap by the real one.
    void document.fonts.load(this.font).then(() => this.onChar());
    this.width = root.clientWidth;
    this.widths = new ResizeObserver(() => this.onWidth());
    this.threads = new ThreadLayer(
      {
        revision,
        sections: this.sections,
        root,
        line: metrics.line,
        estimate: (s, known) => this.heights.estimate(s, known),
        resize: (s, height) => this.heights.resize(s, height),
      },
      listeners.threads,
      collapse,
    );
    this.actions = new ReviewActions(revision, listeners);
    this.heights = new SectionHeights(root, this.sections, metrics, this.threads, mode, collapse);
    // The marks sit beside the scroll bar, in the pane that holds the scrolling root.
    if (root.parentElement) {
      this.marks = new ScrollMarks(root.parentElement, root, () =>
        sectionMarks(this.sections, this.heights.total, this.heights.mode, this.heights.m),
      );
      this.heights.changed = this.marks.refresh;
    }
    this.publishPane();
    this.windowing = new Windowing(root, this.sections, this.heights, {
      mount: (fi) => void this.mount(fi),
      scrolled: () => this.pin.update(this.root.scrollTop),
      unmount: (s, scrollTop) => this.unmount(s, scrollTop),
    });
    let hunks = 0;
    for (const file of files) {
      hunks += file.hunks.length;
      const el = document.createElement("section");
      el.className = "file";
      const index = this.sections.length;
      const header = fileHeader(file, {
        openSource: (line) => listeners.openSource(file.path, line),
        comment: () => this.threads.commentOnFile(this.sections[index]),
        toggleSeen: () => void this.actions.file(this.sections[index]),
        reconfirm: () => void this.actions.reconfirm(this.sections[index]),
        menu: (x, y) => listeners.fileMenu(file.path, x, y),
      });
      const body = document.createElement("div");
      body.className = "body";
      const cards = file.nonText.map((item, ci) =>
        changeCard(file.path, file.oldPath, item, file.review.acknowledged.includes(item.kind), {
          toggle: () => void this.actions.card(this.sections[index], ci),
          comment: () => this.threads.commentOnFile(this.sections[index]),
          blob: (side) => this.source.itemBlob(file.path, item.kind, side),
        }),
      );
      const blockEl = document.createElement("div");
      blockEl.className = "vet-thread-block";
      el.append(header.el, ...cards.map((c) => c.el), blockEl, body);
      el.addEventListener("pointerdown", (e) => this.selectAt(index, e));
      const actions: BarActions = {
        toggleSeen: (h) => void this.actions.hunk(this.sections[index], h),
        comment: (h) => this.threads.commentOnHunk(this.sections[index], h),
        openSource: (h) => listeners.openSource(file.path, file.hunks[h].targetStart),
        toggleBaseDiff: listeners.baseDiff && ((h) => void this.toggleBaseDiff(this.sections[index], h)),
      };
      this.sections.push({
        file,
        review: file.review,
        completion: file.completion,
        el,
        header: header.el,
        body,
        count: header.count,
        meter: header.meter,
        reconfirm: header.reconfirm,
        baseDiffs: new Map(),
        cards,
        top: 0,
        height: 0,
        cardTops: [],
        cardsHeight: 0,
        hunkTops: [],
        ...threadState(blockEl),
        hooks: {
          ...hooks,
          ...bindRevision(hooks, revision),
          ...actions,
          openLine: (line) => listeners.openSource(file.path, line),
        },
        wrap: wrapsByDefault(file.path),
        wrapped: {},
      });
    }
    this.totalHunks = hunks;
    this.stops = new Stops(this.sections.map((s) => ({ cards: s.cards.length, hunks: s.file.hunks.length })));
    this.pin = new StickyHunk(this.sections, metrics, {
      mode: () => this.heights.mode,
      current: (fi, hi) => this.current === this.stopOf(fi, hi),
      press: (fi, hi) => this.select(this.stopOf(fi, hi)),
      pinned: (fi) => listeners.pinned?.(fi === undefined ? undefined : this.sections[fi].file.path),
    });
    this.root.replaceChildren(...(this.threads.lead ? [this.threads.lead] : []), ...this.sections.map((s) => s.el));
    for (const s of this.sections) renderProgress(s, s.review, s.completion);
    this.heights.layout();
    this.pin.update(root.scrollTop);
    this.windowing.start();
    this.root.addEventListener("click", this.heights.onExpand, true);
    this.widths.observe(root);
    inspect?.(
      viewProbe({ sections: this.sections, mounted: this.windowing.mounted, heights: this.heights, stops: this.stops }),
    );
  }

  destroy() {
    this.destroyed = true;
    this.windowing.destroy();
    this.marks?.destroy();
    this.pin.destroy();
    this.widths.disconnect();
    if (this.measuring !== undefined) cancelAnimationFrame(this.measuring);
    this.root.removeEventListener("click", this.heights.onExpand, true);
    for (const fi of this.windowing.mounted) this.sections[fi].handle?.destroy();
    this.windowing.mounted.clear();
    this.threads.destroy();
    this.root.replaceChildren();
  }

  /** Drops the editor; the regions it had expanded are collapsed again when it is mounted anew. */
  private unmount(s: Section, scrollTop = this.root.scrollTop): number {
    s.handle?.destroy();
    s.handle = undefined;
    s.body.textContent = "";
    return this.heights.reset(s, scrollTop);
  }

  setMode(mode: Mode) {
    this.remount(() => {
      this.heights.mode = mode;
      this.publishPane();
      this.measureWraps();
    });
  }

  /** The width of one editor, which the bars inside it size themselves to (`hunkBar.css`). */
  private publishPane() {
    const pane = this.heights.mode === "split" ? this.width / 2 : this.width;
    this.root.style.setProperty("--vet-pane", `${pane}px`);
  }

  /** Drops every editor, lets `change` alter what they show, lays the column out again and mounts what is in view. */
  private remount(change: () => void) {
    const fi = this.heights.indexAt(this.root.scrollTop);
    for (const i of this.windowing.mounted) this.unmount(this.sections[i]);
    this.windowing.mounted.clear();
    change();
    this.heights.layout();
    if (this.sections.length) this.root.scrollTop = this.sections[fi].top;
    this.pin.update(this.root.scrollTop);
    this.pin.refresh();
    this.sync();
  }

  /** The characters that fit a row of the section's code column. */
  private columnsOf(s: Section): number {
    return rowColumns(this.width, this.heights.mode, this.char, Math.max(s.file.baseLines, s.file.targetLines));
  }

  /** The wrapped lines of a section whose text has arrived and whose editor wraps. */
  private wrappedSides(s: Section): WrappedSides {
    if (!s.wrap || !s.loaded) return {};
    const columns = this.columnsOf(s);
    return { base: new WrapRows(s.loaded.base, columns), target: new WrapRows(s.loaded.target, columns) };
  }

  /** Works out again which lines wrap, for the mode, the width and the sections' wrap setting of now. */
  private measureWraps() {
    for (const s of this.sections) s.wrapped = this.wrappedSides(s);
  }

  /**
   * The view's width changed: lines wrap elsewhere. Sections without an editor are estimated again;
   * an editor measures its own.
   */
  private onWidth() {
    const width = this.root.clientWidth;
    if (Math.abs(width - this.width) < 1) return;
    this.width = width;
    this.publishPane();
    this.rewrap();
  }

  /** The font was loaded: the characters may be wider than they were measured. */
  private onChar() {
    const char = charWidth(this.font);
    if (this.destroyed || Math.abs(char - this.char) < 0.01) return;
    this.char = char;
    this.rewrap();
  }

  /** Lines wrap elsewhere: the limit of the marks moves, and the sections without an editor are estimated again. */
  private rewrap() {
    for (const fi of this.windowing.mounted) {
      const s = this.sections[fi];
      s.handle?.dispatchAll(setColumns.of(this.columnsOf(s)));
    }
    if (!this.sections.some((s) => s.wrap)) return;
    const fi = this.heights.indexAt(this.root.scrollTop);
    const offset = this.root.scrollTop - this.sections[fi].top;
    this.measureWraps();
    this.heights.layout(this.windowing.mounted);
    this.root.scrollTop = this.sections[fi].top + offset;
  }

  /** A wrapping editor measured its lines: the section takes the height its editor now has. */
  private heightChanged(s: Section) {
    this.measured.add(s);
    this.measuring ??= requestAnimationFrame(() => {
      this.measuring = undefined;
      let scrollTop = this.root.scrollTop;
      const m = this.heights.m;
      for (const section of this.measured) {
        const views = section.handle?.views;
        if (!views) continue;
        const editor = Math.max(...views.map((v) => v.contentHeight));
        scrollTop += this.heights.resize(
          section,
          m.header + section.cardsHeight + section.block + m.gap + 1 + editor,
          scrollTop,
        );
      }
      this.measured.clear();
    });
  }

  /** Index of the file with this path, if it is in the view. */
  indexOf(path: string): number {
    return this.sections.findIndex((s) => s.file.path === path);
  }

  load(fi: number): Promise<Loaded> {
    const s = this.sections[fi];
    s.loading ??= this.source.contents(s.file).then((loaded) => {
      s.loaded = loaded;
      if (s.wrap) {
        // The text tells how many rows its lines take; the section grows to them before its editor does.
        s.wrapped = this.wrappedSides(s);
        this.heights.resize(s, this.heights.estimate(s));
      }
      // Mounting waits for the next frame, so a large reply's parse and an editor's creation never share one.
      this.windowing.syncNextFrame();
      return loaded;
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
      s.review,
      s.baseDiffs,
      this.heights.mode,
      this.heights.m,
      this.columnsOf(s),
      s.hooks,
      this.collapse ? COLLAPSE : undefined,
      // The pinned header and bar cover the top of the view: a caret scrolled into view stays below them.
      [
        this.threads.extension(s),
        EditorView.scrollMargins.of(() => ({ top: this.heights.m.header + this.heights.m.hunk })),
      ],
      s.wrap ? { heightChanged: () => this.heightChanged(s) } : undefined,
    );
    this.threads.mounted(s);
    this.windowing.mounted.add(fi);
    const first = this.stopOf(fi, 0);
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
    for (const fi of this.windowing.mounted) this.showDiagnostics(this.sections[fi]);
  }

  /** Unmounts far sections and mounts loaded near ones, the ones in view first. */
  sync(wide = true) {
    this.windowing.sync(wide);
  }

  /** Scrolls the file's header to the top and makes its first card or hunk the current stop. */
  async goToFile(fi: number): Promise<Handle> {
    await this.load(fi);
    this.root.scrollTop = this.sections[fi].top;
    const handle = this.mount(fi);
    this.sync();
    const { cards, file } = this.sections[fi];
    if (cards.length + file.hunks.length) this.select(this.stops.start(fi));
    return handle;
  }

  /** Makes `stop` the current one without scrolling. */
  private select(stop: number) {
    if (this.current === stop) return;
    if (this.current >= 0) this.markCurrent(this.current, false);
    this.current = stop;
    this.markCurrent(stop, true);
    this.pin.refresh();
  }

  /** The stop of hunk `hi` of file `fi`. */
  private stopOf(fi: number, hi: number): number {
    return this.stops.start(fi) + this.sections[fi].cards.length + hi;
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
      this.select(this.stops.start(fi) + ci);
      return;
    }
    const views = s.handle?.views ?? [];
    const view = views.find((v) => v.dom.contains(e.target as Node));
    if (!view || !s.file.hunks.length) {
      this.select(this.stops.start(fi));
      return;
    }
    const line = lineOf(view.state.doc, view.posAtCoords({ x: e.clientX, y: e.clientY }, false));
    this.select(this.hunkStopAt(fi, view, line));
  }

  /** The stop of the hunk at or above a 0-based `line` of `view`, one of file `fi`'s editors (a base editor counts base lines). */
  private hunkStopAt(fi: number, view: EditorView, line: number): number {
    const s = this.sections[fi];
    const base = view !== s.handle?.view;
    const hunk = hunkAt(
      s.file.hunks.map((h) => (base ? h.baseStart : h.targetStart)),
      line,
    );
    return this.stops.start(fi) + s.cards.length + hunk;
  }

  /** Shows a stop as current or not: a card outlines itself, a hunk is the editor's. */
  private markCurrent(stop: number, on: boolean) {
    const [fi, si] = this.stops.locate(stop);
    const s = this.sections[fi];
    if (si < s.cards.length) s.cards[si].setCurrent(on);
    else s.handle?.dispatchAll(setCurrent.of(on ? si - s.cards.length : null));
  }

  /** How many stops the view has: its cards and hunks, over all files. */
  get stopCount(): number {
    return this.stops.total;
  }

  /** Goes to the stop with this index over all files' cards and hunks (a card, then the file's hunks). */
  async goToHunk(stop: number): Promise<void> {
    if (!this.stops.total) return;
    stop = this.stops.clamp(stop);
    const [fi, si] = this.stops.locate(stop);
    const s = this.sections[fi];
    const cards = s.cards.length;
    if (si < cards) {
      if (this.current >= 0 && this.current !== stop) this.markCurrent(this.current, false);
      this.current = stop;
      // A card's position is exact, whether or not the section's editor is mounted.
      this.root.scrollTop = Math.max(0, s.top + s.cardTops[si] - this.root.clientHeight / 2);
      this.sync();
      this.markCurrent(stop, true);
      this.pin.refresh();
      return;
    }
    const hi = si - cards;
    const loaded = await this.load(fi);
    const hunk = loaded.hunks[hi];

    if (this.current >= 0 && this.current !== stop) this.markCurrent(this.current, false);
    this.current = stop;

    const handle = this.reveal(fi, hunk.targetStart, hi);
    handle.dispatchAll(setCurrent.of(hi));
    this.pin.refresh();
  }

  /** Where the view is scrolled to, as a file and the distance below its section's top; none for an empty view. */
  position(): ScrollPosition | undefined {
    if (!this.sections.length) return undefined;
    const s = this.sections[this.heights.indexAt(this.root.scrollTop)];
    return { path: s.file.path, offset: this.root.scrollTop - s.top };
  }

  /** Scrolls to `at` if its file is in the view, and starts loading what comes into view; answers whether it is. */
  restore(at: ScrollPosition): boolean {
    const fi = this.indexOf(at.path);
    if (fi < 0) return false;
    this.root.scrollTop = Math.max(0, this.sections[fi].top + at.offset);
    void this.load(fi).catch(() => undefined);
    this.sync();
    return true;
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
   * its hunk), which the backend names, and only the line is revealed.
   */
  async goToSpan(fi: number, side: Side, span: TextSpan): Promise<void> {
    await this.load(fi);
    const { file } = this.sections[fi];
    const path = side === "base" ? (file.oldPath ?? file.path) : file.path;
    // A newer revision replaces this view, and the request fails then.
    const line = await this.source.targetLine(side, path, span.start.line).catch(() => undefined);
    if (line !== undefined && !this.destroyed) this.reveal(fi, line).revealSpan(side, span);
  }

  /** The file and 0-based target line of the focused editor's caret. */
  caret(): { path: string; line: number } | undefined {
    for (const fi of this.windowing.mounted) {
      const { view } = this.sections[fi].handle ?? {};
      if (view && takesKeys(view))
        return {
          path: this.sections[fi].file.path,
          line: lineOf(view.state.doc, view.state.selection.main.head),
        };
    }
    return undefined;
  }

  /** The target line of the first file's caret, focused or not; none while its editor is not mounted. */
  caretLine(): number | undefined {
    const view = this.sections[0]?.handle?.view;
    return view && lineOf(view.state.doc, view.state.selection.main.head);
  }

  /** Where the current stop is, so a view of a newer revision can keep it. */
  currentAnchor(): HunkAnchor | undefined {
    if (this.current < 0 || !this.stops.total) return undefined;
    const [fi, si] = this.stops.locate(this.current);
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
    const hunks = file.hunks.map((h) => h.targetStart);
    this.current = this.stops.nearest(fi, cards.length, card, hunks, anchor.line) ?? this.current;
  }

  /** Renders a file's review state and progress as the backend sent them; a base diff of a hunk that is no longer seen against an old base closes. */
  applyReview(update: ReviewUpdate) {
    const s = this.sections.find((s) => s.file.path === update.path);
    if (!s) return;
    renderReview(s, update);
    for (const hi of s.baseDiffs.keys()) if (s.review.hunks[hi] !== "seenAgainstOldBase") this.setBaseDiff(s, hi);
    this.pin.refresh();
    this.marks?.refresh();
  }

  /** Opens the base diff of a hunk of `s` seen against an old base, or closes it if it is open. */
  private async toggleBaseDiff(s: Section, hi: number) {
    if (s.baseDiffs.has(hi)) {
      this.setBaseDiff(s, hi);
    } else {
      const diff = await this.actions.baseDiff(s, hi);
      if (!diff || this.destroyed || s.review.hunks[hi] !== "seenAgainstOldBase") return;
      this.setBaseDiff(s, hi, diff);
    }
    this.pin.refresh();
  }

  /**
   * Opens hunk `hi`'s base diff of `s` (`diff`) or closes it (none). The section grows or shrinks by
   * exactly the diff's height, wherever it stands in the column.
   */
  private setBaseDiff(s: Section, hi: number, diff?: BaseDiff) {
    const height = (d?: BaseDiff) => (d ? baseDiffHeight(d.rows.length, this.heights.m) : 0);
    const delta = height(diff) - height(s.baseDiffs.get(hi));
    if (diff) s.baseDiffs.set(hi, diff);
    else s.baseDiffs.delete(hi);
    s.hunkTops = s.hunkTops.map((top, j) => (j > hi ? top + delta : top));
    s.handle?.dispatchAll(setBaseDiffs.of(new Map(s.baseDiffs)));
    this.heights.resize(s, s.height + delta);
  }

  /** Asks the backend to flip the current stop: a hunk or a card. */
  toggleCurrent(): Promise<void> {
    const [fi, si] = this.stops.locate(this.current);
    const s = this.sections[fi];
    return si < s.cards.length ? this.actions.card(s, si) : this.actions.hunk(s, si - s.cards.length);
  }

  /**
   * Makes the hunk at the focused editor's caret (else the nearest one above it) the current stop.
   * Undefined when no editor of a file with hunks has the focus.
   */
  private selectAtCaret(): number | undefined {
    for (const fi of this.windowing.mounted) {
      const s = this.sections[fi];
      if (!s.file.hunks.length) continue;
      const view = s.handle?.views.find(takesKeys);
      if (!view) continue;
      const stop = this.hunkStopAt(fi, view, lineOf(view.state.doc, view.state.selection.main.head));
      this.select(stop);
      return stop;
    }
    return undefined;
  }

  /** Selects the hunk at the caret and asks the backend to flip it; undefined when no editor has the focus. */
  toggleAtCaret(): Promise<void> | undefined {
    return this.selectAtCaret() === undefined ? undefined : this.toggleCurrent();
  }

  /**
   * Re-confirms the hunk at the focused editor's caret, else the current stop, if it is seen against
   * an old base. Undefined when there is no such hunk.
   */
  reconfirmCurrent(): Promise<void> | undefined {
    const stop = this.selectAtCaret() ?? (this.current >= 0 ? this.current : undefined);
    if (stop === undefined || !this.stops.total) return undefined;
    const [fi, si] = this.stops.locate(stop);
    const s = this.sections[fi];
    const hi = si - s.cards.length;
    return hi >= 0 && s.review.hunks[hi] === "seenAgainstOldBase" ? this.actions.hunk(s, hi) : undefined;
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
    if (this.threads.commentOnSelection([...this.windowing.mounted].map((fi) => this.sections[fi]))) return true;
    if (this.current < 0 || !this.stops.total) return false;
    const [fi, si] = this.stops.locate(this.current);
    this.threads.commentOnStop(this.sections[fi], si);
    return true;
  }

  /** Opens the form for a comment on the `hunk`-th hunk of a one-file view. */
  commentOnHunk(hunk: number) {
    this.threads.commentOnHunk(this.sections[0], hunk);
  }

  /** Scrolls to where a thread is shown; answers whether it is in this view. */
  async goToThread(thread: Thread): Promise<boolean> {
    const at = this.threads.find(thread);
    if (!at) return false;
    if (at === "lead") this.root.scrollTop = 0;
    else if (at.line !== undefined) await this.goToLine(at.index, at.line);
    else await this.goToFile(at.index);
    return true;
  }
}
