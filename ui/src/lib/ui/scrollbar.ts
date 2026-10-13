// Vet's own vertical scroll bar: a gutter beside the content it scrolls, which always shows and
// takes its own width, with the thumb a translucent band. A view can have marks drawn in the
// gutter under the thumb (the review views mark their hunks), so the marks stay visible. The
// scroller keeps scrolling by wheel, gesture and key itself; its platform bar is hidden
// (`.vet-scroller` in `global.css`).

/** A mark in the content, in px from its top; `kind` picks its class, `vet-scrollmark-<kind>`. */
export interface ScrollMark {
  top: number;
  height: number;
  kind: string;
}

/** Where the marks lie in content `total` px tall. */
export interface MarkLayout {
  total: number;
  marks: readonly ScrollMark[];
}

export interface TrackRect {
  top: number;
  height: number;
}

/** A mark's smallest size on the track, so a one-line hunk of a long changeset can still be seen. */
export const MIN_MARK = 3;
/** The thumb's smallest size, so it can still be grabbed in long content. */
export const MIN_THUMB = 20;

/** The marks scaled to a track of `track` px, none smaller than `MIN_MARK` and none off the track. */
export function markRects(layout: MarkLayout, track: number): (TrackRect & { kind: string })[] {
  if (layout.total <= 0 || track <= 0) return [];
  const scale = track / layout.total;
  return layout.marks.map((m) => {
    const height = Math.min(Math.max(m.height * scale, MIN_MARK), track);
    const top = Math.min(Math.max(m.top * scale, 0), track - height);
    return { top, height, kind: m.kind };
  });
}

/** The thumb for `viewport` px in view of `content` px scrolled by `scrollTop`; none when it all fits. */
export function thumbRect(content: number, viewport: number, scrollTop: number, track: number): TrackRect {
  const range = content - viewport;
  if (range < 1 || track <= 0) return { top: 0, height: 0 };
  const height = Math.min(Math.max((viewport / content) * track, MIN_THUMB), track);
  const top = (Math.min(Math.max(scrollTop, 0), range) / range) * (track - height);
  return { top, height };
}

/** The scroll offset that puts a thumb `thumb` px tall at `top` on the track. */
export function scrollForThumb(content: number, viewport: number, track: number, thumb: number, top: number): number {
  const free = track - thumb;
  if (free <= 0) return 0;
  return (Math.min(Math.max(top, 0), free) / free) * Math.max(content - viewport, 0);
}

/** What a press on the track scrolls: a page less a little overlap, so the reader keeps their place. */
function pageStep(viewport: number): number {
  return Math.max(viewport * 0.9, 1);
}

// A press on the track pages once, then again at this pace while the button stays down.
const REPEAT_DELAY = 300;
const REPEAT_EVERY = 60;
// A wheel's delta in lines or pages, in px.
const WHEEL_LINE = 16;

export interface ScrollbarOptions {
  /** Marks to draw under the thumb; asked again after `refresh`. */
  marks?: () => MarkLayout | undefined;
  /**
   * Follows the content's size by observing the scroller's children (on by default). An owner
   * that knows when its content changes height turns it off and calls `refresh`.
   */
  watchContent?: boolean;
}

/**
 * The bar of `scroller`. Its owner places `dom` beside the scroller (a flex item of `--w-scrollbar`)
 * and calls `destroy` when the scroller goes.
 */
export class Scrollbar {
  readonly dom = document.createElement("div");
  private thumb = document.createElement("div");
  private markEls: HTMLElement[] = [];
  private layout?: MarkLayout;
  private frame?: number;
  private resize = new ResizeObserver(() => this.schedule());
  private children?: MutationObserver;
  private drag?: { y: number; top: number };
  private repeat?: number;
  private scroller: HTMLElement;
  private options: ScrollbarOptions;

  constructor(scroller: HTMLElement, options: ScrollbarOptions = {}) {
    this.scroller = scroller;
    this.options = options;
    this.dom.className = "vet-scrollbar";
    this.dom.setAttribute("aria-hidden", "true");
    this.thumb.className = "vet-scrollbar-thumb";
    this.dom.append(this.thumb);
    scroller.classList.add("vet-scroller");
    scroller.addEventListener("scroll", this.schedule, { passive: true });
    this.dom.addEventListener("pointerdown", this.press);
    this.dom.addEventListener("wheel", this.wheel, { passive: false });
    this.resize.observe(scroller);
    this.resize.observe(this.dom);
    if (options.watchContent !== false) {
      for (const child of scroller.children) this.resize.observe(child);
      this.children = new MutationObserver((records) => {
        for (const r of records) {
          for (const n of r.addedNodes) if (n instanceof Element) this.resize.observe(n);
          for (const n of r.removedNodes) if (n instanceof Element) this.resize.unobserve(n);
        }
        this.schedule();
      });
      this.children.observe(scroller, { childList: true });
    }
    this.schedule();
  }

  /** The content or the marks changed: draws them again in the next frame. */
  refresh = () => {
    this.layout = undefined;
    this.schedule();
  };

  private schedule = () => {
    this.frame ??= requestAnimationFrame(() => {
      this.frame = undefined;
      this.draw();
    });
  };

  private metrics() {
    const { scrollHeight, clientHeight, scrollTop } = this.scroller;
    const track = this.dom.clientHeight;
    return { content: scrollHeight, viewport: clientHeight, scrollTop, track };
  }

  private draw() {
    const { content, viewport, scrollTop, track } = this.metrics();
    if (this.options.marks) {
      this.layout ??= this.options.marks();
      const rects = this.layout ? markRects(this.layout, track) : [];
      while (this.markEls.length < rects.length) {
        const mark = document.createElement("div");
        this.dom.insertBefore(mark, this.thumb);
        this.markEls.push(mark);
      }
      // The nodes of marks that are gone leave with them.
      while (this.markEls.length > rects.length) this.markEls.pop()?.remove();
      this.markEls.forEach((mark, i) => {
        const rect = rects.at(i);
        mark.hidden = !rect;
        if (!rect) return;
        mark.className = `vet-scrollmark vet-scrollmark-${rect.kind}`;
        mark.style.top = `${rect.top}px`;
        mark.style.height = `${rect.height}px`;
      });
    }
    const thumb = thumbRect(content, viewport, scrollTop, track);
    this.thumb.hidden = thumb.height <= 0;
    this.thumb.style.top = `${thumb.top}px`;
    this.thumb.style.height = `${thumb.height}px`;
  }

  private press = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const { content, viewport, scrollTop, track } = this.metrics();
    const thumb = thumbRect(content, viewport, scrollTop, track);
    if (thumb.height <= 0) return;
    const y = e.clientY - this.dom.getBoundingClientRect().top;
    this.dom.setPointerCapture(e.pointerId);
    this.dom.addEventListener("pointerup", this.release);
    this.dom.addEventListener("pointercancel", this.release);
    if (y >= thumb.top && y < thumb.top + thumb.height) {
      this.drag = { y: e.clientY, top: thumb.top };
      this.dom.classList.add("dragging");
      this.dom.addEventListener("pointermove", this.move);
      return;
    }
    // Pages towards the pointer until the thumb reaches it.
    const page = () => {
      const m = this.metrics();
      const t = thumbRect(m.content, m.viewport, m.scrollTop, m.track);
      if (y >= t.top && y < t.top + t.height) {
        this.stopRepeat();
        return;
      }
      this.scroller.scrollBy({ top: (y < t.top ? -1 : 1) * pageStep(m.viewport), behavior: "instant" });
    };
    page();
    this.repeat = window.setTimeout(() => {
      this.repeat = window.setInterval(page, REPEAT_EVERY);
    }, REPEAT_DELAY);
  };

  private move = (e: PointerEvent) => {
    if (!this.drag) return;
    const { content, viewport, scrollTop, track } = this.metrics();
    const { height } = thumbRect(content, viewport, scrollTop, track);
    this.scroller.scrollTop = scrollForThumb(content, viewport, track, height, this.drag.top + e.clientY - this.drag.y);
  };

  private release = (e: PointerEvent) => {
    this.dom.releasePointerCapture(e.pointerId);
    this.dom.removeEventListener("pointermove", this.move);
    this.dom.removeEventListener("pointerup", this.release);
    this.dom.removeEventListener("pointercancel", this.release);
    this.dom.classList.remove("dragging");
    this.drag = undefined;
    this.stopRepeat();
  };

  private stopRepeat() {
    // A timeout's and an interval's ids share one pool, and either clear call clears both kinds.
    clearTimeout(this.repeat);
    this.repeat = undefined;
  }

  /** The wheel over the gutter scrolls the content, as it would over the content itself. */
  private wheel = (e: WheelEvent) => {
    const unit =
      e.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? WHEEL_LINE
        : e.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? this.scroller.clientHeight
          : 1;
    e.preventDefault();
    this.scroller.scrollBy({ top: e.deltaY * unit, left: e.deltaX * unit, behavior: "instant" });
  };

  destroy() {
    if (this.frame !== undefined) cancelAnimationFrame(this.frame);
    this.stopRepeat();
    this.resize.disconnect();
    this.children?.disconnect();
    this.scroller.removeEventListener("scroll", this.schedule);
    this.scroller.classList.remove("vet-scroller");
    this.dom.remove();
  }
}
