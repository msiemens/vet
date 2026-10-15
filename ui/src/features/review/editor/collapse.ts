// The collapsed regions of one editor side ("N unchanged lines" bars), owned by the editor in a state
// field instead of read back from CodeMirror's decorations. The ranges are the ones `@codemirror/merge`
// collapses, computed from its public changed chunks; a click expands a range on both sides of a merge
// view, as its own collapse widget did.

import { type Chunk, getChunks, mergeViewSiblings } from "@codemirror/merge";
import { type EditorState, RangeSetBuilder, StateEffect, StateField, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import type { Span } from "./hiddenRanges";

/** Unchanged regions collapse to a bar that expands on click: `margin` lines of context stay around each change, and a bar hides at least `minSize` lines. */
export interface Collapse {
  margin: number;
  minSize: number;
}

/** The regions `state` hides behind a bar, by the same rule as `collapseUnchanged` of `@codemirror/merge`. */
export function collapsedSpans(state: EditorState, collapse: Collapse): Span[] {
  const chunks = getChunks(state);
  if (!chunks) return [];
  const isA = chunks.side === "a";
  const spans: Span[] = [];
  const { doc } = state;
  let prevLine = 1;
  for (let i = 0; ; i++) {
    const chunk = i < chunks.chunks.length ? chunks.chunks[i] : null;
    const fromLine = i ? prevLine + collapse.margin : 1;
    const toLine = chunk ? doc.lineAt(isA ? chunk.fromA : chunk.fromB).number - 1 - collapse.margin : doc.lines;
    if (toLine - fromLine + 1 >= collapse.minSize)
      spans.push({ from: doc.line(fromLine).from, to: doc.line(toLine).to });
    if (!chunk) break;
    prevLine = doc.lineAt(Math.min(doc.length, isA ? chunk.toA : chunk.toB)).number;
  }
  return spans;
}

/** Replaces the collapsed regions of a side, whole. */
export const setCollapsed = StateEffect.define<readonly Span[]>();
/** Expands the collapsed region that starts at the given position. */
export const expandCollapsed = StateEffect.define<number>({ map: (value, change) => change.mapPos(value) });

/** The collapsed regions of one editor side; empty where collapsing is off. */
export const collapsedField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(bars, tr) {
    bars = bars.map(tr.changes);
    let wanted: readonly Span[] | undefined;
    for (const e of tr.effects) {
      if (e.is(setCollapsed)) wanted = e.value;
      else if (e.is(expandCollapsed)) bars = bars.update({ filter: (from) => from !== e.value });
    }
    return wanted ? buildBars(wanted, tr.state.doc) : bars;
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** The regions hidden behind an "N unchanged lines" bar of `state`. */
export function hiddenRegions(state: EditorState): Span[] {
  const bars = state.field(collapsedField, false);
  if (!bars?.size) return [];
  const regions: Span[] = [];
  bars.between(0, state.doc.length, (from, to) => {
    regions.push({ from, to });
  });
  return regions;
}

/** The bars a side should start with, given the merge view it is part of. */
export function initialCollapse(view: EditorView, collapse: Collapse): void {
  view.dispatch({ effects: setCollapsed.of(collapsedSpans(view.state, collapse)) });
}

function buildBars(spans: readonly Span[], doc: Text): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of spans) {
    const lines = doc.lineAt(to).number - doc.lineAt(from).number + 1;
    builder.add(from, to, Decoration.replace({ widget: new CollapseBar(lines), block: true }));
  }
  return builder.finish();
}

/** The height a bar reports before it is measured, in px. */
const BAR_HEIGHT = 27;

/** The bar over a collapsed region; its click expands the region here and on the sibling editor. */
class CollapseBar extends WidgetType {
  private readonly lines: number;

  constructor(lines: number) {
    super();
    this.lines = lines;
  }

  eq(other: WidgetType) {
    return other instanceof CollapseBar && other.lines === this.lines;
  }

  toDOM(view: EditorView) {
    const bar = document.createElement("div");
    bar.className = "cm-collapsedLines";
    bar.textContent = view.state.phrase("$ unchanged lines", this.lines);
    bar.addEventListener("click", () => expand(view, view.posAtDOM(bar)));
    return bar;
  }

  ignoreEvent(e: Event) {
    return e instanceof MouseEvent;
  }

  get estimatedHeight() {
    return BAR_HEIGHT;
  }
}

/** Expands the collapsed region at `pos` here, and the one it corresponds to on the other side of a merge view. */
function expand(view: EditorView, pos: number) {
  view.dispatch({ effects: expandCollapsed.of(pos) });
  const siblings = mergeViewSiblings(view);
  const chunks = getChunks(view.state);
  if (!siblings || !chunks) return;
  const other = siblings.a === view ? siblings.b : siblings.a;
  other.dispatch({ effects: expandCollapsed.of(mapPos(pos, chunks.chunks, chunks.side === "a")) });
}

/** The same position on the other side of a merge view, moved by the changed chunks. */
function mapPos(pos: number, chunks: readonly Chunk[], isA: boolean): number {
  let here = 0;
  let there = 0;
  for (let i = 0; ; i++) {
    const next = i < chunks.length ? chunks[i] : null;
    if (!next || (isA ? next.fromA : next.fromB) >= pos) return there + (pos - here);
    [here, there] = isA ? [next.toA, next.toB] : [next.toB, next.toA];
  }
}
