// The height model of a `ChangesetView`: where each section starts and how tall it is. A height
// is estimated from the manifest (`sectionLayout`), so scrolling does not reflow; what is measured
// (a thread card, an expanded region) corrects it, and the scroll position follows when the
// correction happens above the viewport.

import type { Mode } from "../../../lib/viewMode.ts";
import { lineOf } from "./positions.ts";
import type { Section } from "./section.ts";
import { type Metrics, hiddenAt, sectionLayout } from "./sectionLayout.ts";
import type { ThreadLayer } from "./threadLayer.ts";

const px = (style: CSSStyleDeclaration, name: string) => parseFloat(style.getPropertyValue(name));

/** The row heights of the theme's tokens, as `root` has them. */
export function readMetrics(root: HTMLElement): Metrics {
  const style = getComputedStyle(root);
  return {
    line: px(style, "--h-line"),
    hunk: px(style, "--h-hunk"),
    header: px(style, "--h-section"),
    collapsed: px(style, "--h-collapsed"),
    gap: px(style, "--h-section-gap"),
    preview: px(style, "--h-preview"),
  };
}

/** The index of the last section starting at or above `y` (the first one above all of them). */
export function sectionAt(sections: readonly { top: number }[], y: number): number {
  let lo = 0;
  let hi = sections.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (sections[mid].top <= y) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

export class SectionHeights {
  private root: { scrollTop: number };
  private sections: Section[];
  readonly m: Metrics;
  private threads: ThreadLayer;
  mode: Mode;
  /** Collapse unchanged regions (the Changes tab); otherwise the full file is shown. */
  private collapse: boolean;

  constructor(
    root: { scrollTop: number },
    sections: Section[],
    m: Metrics,
    threads: ThreadLayer,
    mode: Mode,
    collapse: boolean,
  ) {
    this.root = root;
    this.sections = sections;
    this.m = m;
    this.threads = threads;
    this.mode = mode;
    this.collapse = collapse;
  }

  get total(): number {
    const last = this.sections.at(-1);
    return last ? last.top + last.height : 0;
  }

  /** Estimates every section's height; the section's own bottom border is one more pixel. */
  layout() {
    let top = this.threads.leadHeight;
    for (const s of this.sections) {
      s.top = top;
      s.height = this.estimate(s);
      s.el.style.height = `${s.height}px`;
      top += s.height;
    }
  }

  /** Lays the section out from the manifest; its own bottom border is one more pixel. `known`: see `ThreadLayer.count`. */
  estimate(s: Section, known = new Map<string, number>()): number {
    const space = this.threads.space(s, known);
    const layout = sectionLayout(s.file, this.mode, this.m, this.collapse, space);
    s.hunkTops = layout.hunkTops;
    s.cardTops = layout.cardTops;
    s.cardsHeight = layout.cardsHeight;
    this.threads.count(s, space, layout.drawn, known);
    s.el.style.setProperty("--vet-cards", `${layout.cardsHeight}px`);
    return layout.height + 1;
  }

  /**
   * Sets a section's height; sections above the viewport shift what is shown, so the scroll position follows.
   * `scrollTop` is the caller's reading of it: a read after the height writes of a loop forces a layout each time.
   * Returns how far the scroll position moved.
   */
  resize(s: Section, height: number, scrollTop = this.root.scrollTop): number {
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

  /** Back to the estimate, as when the section's editor is dropped. */
  reset(s: Section, scrollTop = this.root.scrollTop): number {
    return this.resize(s, this.estimate(s), scrollTop);
  }

  /** The index of the section at height `y` of the column. */
  indexAt(y: number): number {
    return sectionAt(this.sections, y);
  }

  /**
   * Expanding a collapsed region reveals the lines the bar stood for, which is exact; the
   * editor's own height is not a reliable measure, as CodeMirror estimates what is off screen.
   */
  onExpand = (e: MouseEvent) => {
    const bar = e.target instanceof Element ? e.target.closest(".cm-collapsedLines") : null;
    if (!bar) return;
    const s = this.sections.find((s) => s.el.contains(bar));
    const view = s?.handle?.views.find((v) => v.dom.contains(bar));
    if (!s || !view) return;
    // The bar sits where the hidden lines start, which is also where the merge extension keys it.
    const line = lineOf(view.state.doc, view.posAtDOM(bar));
    const lines = hiddenAt(s.file, this.collapse, view === s.handle?.view ? "target" : "base", line);
    if (lines) this.resize(s, s.height + lines * this.m.line - this.m.collapsed);
  };
}
