// Which sections of a `ChangesetView` have an editor: only those near the viewport. Creating an
// editor is the expensive step, so a frame mounts only as many as fit a time budget and the rest
// follow on later frames. A frame has one such pass.

import { closeHoverTooltips, showTooltip } from "@codemirror/view";
import { Debounced } from "../../../lib/debounced.ts";
import type { Section } from "./section.ts";
import type { SectionHeights } from "./sectionHeights.ts";

/** Time a frame may spend creating editors before the rest wait for the next frame. */
const MOUNT_BUDGET_MS = 6;
/** Scrolling counts as stopped after this long without a scroll event. */
const SETTLE_MS = 120;
/** Acting in the view (a press, a key) keeps background work waiting for this long. */
const ACT_MS = 200;

/** An inclusive range of section indices. */
export type Range = [first: number, last: number];

/**
 * The sections that get an editor (`near`) and the ones that keep one (`far`) for a viewport of
 * height `h` at `top`. While scrolling (`wide` off) only what is in view is near; the surroundings
 * join once the scrolling stops.
 */
export function windowRanges(indexAt: (y: number) => number, top: number, h: number, wide: boolean) {
  const near: Range = wide ? [indexAt(Math.max(0, top - h)), indexAt(top + 2 * h)] : [indexAt(top), indexAt(top + h)];
  const far: Range = [indexAt(Math.max(0, top - 2 * h)), indexAt(top + 3 * h)];
  return { near, far };
}

export interface WindowHost {
  /** Asks for the contents of a section near the viewport, so it can be shown there. */
  load(fi: number): void;
  /** Creates the editor of a loaded section. */
  mount(fi: number): void;
  /** The root scrolled. */
  scrolled(): void;
  /** Drops a section's editor; returns how far the scroll position moved. */
  unmount(s: Section, scrollTop: number): number;
  /** A section near the viewport is looked at: whatever it drew may be out of date. */
  check(fi: number): void;
}

export class Windowing {
  /** The sections with an editor. */
  readonly mounted = new Set<number>();
  /** Sections whose cards show their image previews. */
  private previewing = new Set<number>();
  private settle = new Debounced();
  /** Set from a scroll event until scrolling settles. */
  private scrolling = false;
  /** Set while the viewer acts in the view, until the actions pause. */
  private acting = false;
  private acted = new Debounced();
  /** A scroll event ran this frame's pass. */
  private passed = false;
  private endOfPass?: number;
  /** The frame request that continues mounting once a frame's budget is spent. */
  private later?: number;
  private destroyed = false;
  private root: HTMLElement;
  private sections: Section[];
  private heights: SectionHeights;
  private host: WindowHost;

  constructor(root: HTMLElement, sections: Section[], heights: SectionHeights, host: WindowHost) {
    this.root = root;
    this.sections = sections;
    this.heights = heights;
    this.host = host;
  }

  /** Listens to the root's scroll and to what the viewer does in it. Called before any editor mounts: see `onScroll`. */
  start() {
    this.root.addEventListener("scroll", this.onScroll, { passive: true });
    this.root.addEventListener("pointerdown", this.onAct, { passive: true });
    this.root.addEventListener("keydown", this.onAct);
  }

  /** The viewer is scrolling or has just scrolled or acted: background loading waits for them to pause. */
  get busy(): boolean {
    return this.scrolling || this.acting;
  }

  destroy() {
    this.destroyed = true;
    if (this.later !== undefined) cancelAnimationFrame(this.later);
    if (this.endOfPass !== undefined) cancelAnimationFrame(this.endOfPass);
    this.later = this.endOfPass = undefined;
    this.settle.cancel();
    this.acted.cancel();
    this.root.removeEventListener("scroll", this.onScroll);
    this.root.removeEventListener("pointerdown", this.onAct);
    this.root.removeEventListener("keydown", this.onAct);
    for (const fi of this.previewing) this.sections[fi].cards.forEach((c) => c.setPreview(false));
    this.previewing.clear();
  }

  /** A press or a key in the view: the trickle waits until they pause. */
  private onAct = () => {
    this.acting = true;
    this.acted.schedule(ACT_MS, () => (this.acting = false));
  };

  /** While the viewer scrolls only what is in view mounts; the surroundings follow once the scrolling stops. */
  private onScroll = (e: Event) => {
    // CodeMirror measures each of its editors, synchronously, on a scroll of any ancestor. This
    // listener is registered before any editor mounts, so it runs first and keeps them from it;
    // `measureVisible` asks the visible ones for one measure in this frame's batch instead.
    e.stopImmediatePropagation();
    this.scrolling = true;
    this.host.scrolled();
    this.measureVisible();
    // Editors mounted here measure in this frame's batch. The pass is this frame's: a continuation
    // that is due in it gives way (see `syncNextFrame`).
    this.passed = true;
    this.sync(false);
    this.endOfPass ??= requestAnimationFrame(() => {
      this.endOfPass = undefined;
      this.passed = false;
    });
    this.settle.schedule(SETTLE_MS, () => {
      this.scrolling = false;
      this.sync();
    });
  };

  /**
   * Unmounts far sections and mounts loaded near ones, the ones in view first, as far as the frame's
   * budget goes; the rest follows in the next frames.
   */
  sync(wide = true) {
    // A load that finished after the view was replaced; an empty changeset has no section to mount.
    if (this.destroyed || !this.sections.length) return;
    let top = this.root.scrollTop;
    const h = this.root.clientHeight;
    const { near, far } = windowRanges((y) => this.heights.indexAt(y), top, h, wide);
    // What is in view and the margin around it loads first; the rest is the owner's to trickle.
    for (let fi = near[0]; fi <= near[1]; fi++) this.host.load(fi);
    for (const fi of [...this.mounted]) {
      if (fi < far[0] || fi > far[1]) {
        top += this.host.unmount(this.sections[fi], top);
        this.mounted.delete(fi);
      }
    }
    for (let fi = near[0]; fi <= near[1]; fi++) {
      this.host.check(fi);
      this.setPreviews(fi, true);
    }
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
        this.syncNextFrame();
        return;
      }
      this.host.mount(fi);
    }
  }

  /**
   * Continues mounting in a later frame. A frame has one pass: when a scroll event already ran it,
   * the continuation due in the same frame waits for the next.
   */
  syncNextFrame() {
    this.later ??= requestAnimationFrame(() => {
      this.later = undefined;
      if (this.passed) this.syncNextFrame();
      else this.sync(!this.scrolling);
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
}
