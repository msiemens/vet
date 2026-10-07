// A scrolling column of file sections: the whole changeset (the Changes tab) or one
// file (a file tab). Section heights are estimated up front from the manifest, so
// scrolling does not reflow, and corrected once the editor is mounted. An editor is
// mounted only for sections near the viewport.

import { EditorView } from "@codemirror/view";
import type { FileChange } from "../../../generated/FileChange";
import type { NonTextKind } from "../../../generated/NonTextKind";
import type { ReviewUpdate } from "../../../generated/ReviewUpdate";
import type { Side } from "../../../generated/Side";
import type { Thread } from "../../../generated/Thread";
import type { TextSpan } from "../../../generated/TextSpan";
import type { Mode } from "../../../lib/viewMode";
import { changeCard } from "./changeCard";
import { takesKeys } from "./dom";
import { fileHeader, renderCount } from "./fileHeader";
import type { FileSource } from "./fileSource";
import type { BarActions } from "./hunkBar";
import { type Handle, type Hooks, type Loaded, mountFile } from "./mount";
import { lineOf, lineStart } from "./positions";
import { type ReviewRequests, ReviewActions, renderReview } from "./reviewActions";
import { setCurrent } from "./reviewState";
import { StickyHunk } from "./stickyHunk";
import { Stops, hunkAt } from "./stops";
import type { Section } from "./section";
import { SectionHeights, readCharWidth, readMetrics } from "./sectionHeights";
import { COLLAPSE, type WrappedSides } from "./sectionLayout";
import { type SemanticHooks, type SemanticSource, bindRevision } from "./semantic";
import { wrapsByDefault } from "./softWrap";
import { type ThreadListeners, ThreadLayer, threadState } from "./threadLayer";
import { type ViewProbe, viewProbe } from "./viewProbe";
import { Windowing } from "./windowing";
import { WrapRows, rowColumns } from "./wrapRows";

/** The editor hooks as the owner provides them: the semantic requests still name the view's revision. */
export type ViewHooks = Omit<Hooks, keyof BarActions | keyof SemanticHooks> & SemanticSource;

export interface Listeners extends ReviewRequests {
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
  /** The width of a character, and the view's width: what lines wrap at. */
  private char: number;
  private width: number;
  private widths: ResizeObserver;
  /** Wrapping sections whose editor measured another height than the section has. */
  private measured = new Set<Section>();
  private measuring?: number;

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
    /** Soft wrap for every file; absent: each file by its kind (`wrapsByDefault`). */
    private wrapAll?: boolean,
    /** Hands the dev tools what they measure; production owners leave it out. */
    inspect?: (probe: ViewProbe) => void,
  ) {
    // Thread cards size themselves to the view's width (`cqw`), not to the width of the longest line.
    root.style.containerType = "inline-size";
    const metrics = readMetrics(root);
    this.char = readCharWidth(root);
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
        openDiff: listeners.openDiff && ((line) => listeners.openDiff?.(file.path, line)),
        comment: () => this.threads.commentOnFile(this.sections[index]),
        toggleSeen: () => void this.actions.file(this.sections[index]),
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
      };
      this.sections.push({
        file,
        review: file.review,
        completion: file.completion,
        el,
        header: header.el,
        body,
        count: header.count,
        cards,
        top: 0,
        height: 0,
        cardTops: [],
        cardsHeight: 0,
        hunkTops: [],
        ...threadState(blockEl),
        hooks: { ...hooks, ...bindRevision(hooks, revision), ...actions },
        wrap: this.wrapAll ?? wrapsByDefault(file.path),
        wrapped: {},
      });
    }
    this.totalHunks = hunks;
    this.stops = new Stops(this.sections.map((s) => ({ cards: s.cards.length, hunks: s.file.hunks.length })));
    this.pin = new StickyHunk(this.sections, metrics, {
      mode: () => this.heights.mode,
      current: (fi, hi) => this.current === this.stopOf(fi, hi),
      press: (fi, hi) => this.select(this.stopOf(fi, hi)),
    });
    this.root.replaceChildren(...(this.threads.lead ? [this.threads.lead] : []), ...this.sections.map((s) => s.el));
    for (const s of this.sections) renderCount(s, s.completion);
    this.heights.layout();
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
      this.measureWraps();
    });
  }

  /** Whether the view wraps long lines: as set, else if every file does by its kind. */
  wrapped(): boolean {
    return this.wrapAll ?? (this.sections.length > 0 && this.sections.every((s) => s.wrap));
  }

  /** Wraps long lines in every file, or scrolls them sideways. */
  setWrap(on: boolean) {
    this.wrapAll = on;
    this.remount(() => {
      for (const s of this.sections) s.wrap = on;
      this.measureWraps();
    });
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

  /** The wrapped lines of a section whose text has arrived and whose editor wraps. */
  private wrappedSides(s: Section): WrappedSides {
    if (!s.wrap || !s.loaded) return {};
    const columns = rowColumns(
      this.width,
      this.heights.mode,
      this.char,
      Math.max(s.file.baseLines, s.file.targetLines),
    );
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
      s.review.hunks,
      this.heights.mode,
      this.heights.m.hunk,
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

  /** Renders a file's review state and progress as the backend sent them. */
  applyReview(update: ReviewUpdate) {
    const s = this.sections.find((s) => s.file.path === update.path);
    if (!s) return;
    renderReview(s, update);
    this.pin.refresh();
  }

  /** Asks the backend to flip the current stop: a hunk or a card. */
  toggleCurrent(): Promise<void> {
    const [fi, si] = this.stops.locate(this.current);
    const s = this.sections[fi];
    return si < s.cards.length ? this.actions.card(s, si) : this.actions.hunk(s, si - s.cards.length);
  }

  /**
   * Makes the hunk at the focused editor's caret (else the nearest one above it) the current stop
   * and asks the backend to flip it. Undefined when no editor of a file with hunks has the focus.
   */
  toggleAtCaret(): Promise<void> | undefined {
    for (const fi of this.windowing.mounted) {
      const s = this.sections[fi];
      if (!s.file.hunks.length) continue;
      const view = s.handle?.views.find(takesKeys);
      if (!view) continue;
      this.select(this.hunkStopAt(fi, view, lineOf(view.state.doc, view.state.selection.main.head)));
      return this.toggleCurrent();
    }
    return undefined;
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
