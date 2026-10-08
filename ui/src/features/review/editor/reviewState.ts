// Review-state decorations on every changed line (including the deleted lines of
// the unified view) and hunk header bars (SPEC §10).

import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { StateEffect, StateField, type Extension, type Range, type Text } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  GutterMarker,
  ViewPlugin,
  type ViewUpdate,
  gutter,
} from "@codemirror/view";
import type { BaseDiff } from "../../../generated/BaseDiff";
import type { FileReview } from "../../../generated/FileReview";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { replacesBaseTail } from "./baseTail";
import { BaseDiffBlock } from "./baseDiffBlock";
import { gutterWidth } from "./gutterWidth";
import { type BarActions, HunkBar, hunkHeader } from "./hunkBar";
import { lineEnd, lineStart } from "./positions";
import type { Metrics } from "./sectionLayout";

export const setReview = StateEffect.define<FileReview>();
/** The base diffs open under hunk bars, by hunk. */
export const setBaseDiffs = StateEffect.define<ReadonlyMap<number, BaseDiff>>();
export const setCurrent = StateEffect.define<number | null>();

const CLASS: Record<HunkState, string> = {
  unseen: "vet-unseen",
  seen: "vet-seen",
  seenAgainstOldBase: "vet-old",
  informational: "vet-info",
  unapplied: "vet-unapplied",
};

interface Span {
  from: number;
  to: number;
  empty: boolean;
  /** Start of the line where this hunk sits in the document: where a unified view places a deleted block. */
  at: number;
}

export interface ReviewState {
  spans: Span[];
  states: HunkState[];
  /** Per hunk, the lines seen against an old base only. */
  oldBase: number[];
  diffs: ReadonlyMap<number, BaseDiff>;
  current: number | null;
  decos: DecorationSet;
}

/** The sizes of a bar and of the base diff under it, in px. */
export type BarMetrics = Pick<Metrics, "hunk" | "line" | "diffGap">;

export interface SideConfig {
  /** `[start, length]` of each hunk on this side, 0-based lines. */
  lines: [number, number][];
  headers: string[];
  /** Absent on the base side of a side-by-side view. */
  bar: BarActions | null;
  m: BarMetrics;
}

export function sideConfig(hunks: Hunk[], side: "base" | "target", bar: BarActions | null, m: BarMetrics): SideConfig {
  return {
    lines: hunks.map((h) => (side === "base" ? [h.baseStart, h.baseLen] : [h.targetStart, h.targetLen])),
    headers: hunks.map(hunkHeader),
    bar,
    m,
  };
}

const gutterMarkers: Record<string, GutterMarker> = {};
for (const cls of Object.values(CLASS)) {
  gutterMarkers[cls] = new (class extends GutterMarker {
    elementClass = cls;
  })();
}

/** The strip's marker for a changed line of a hunk in `state`. */
export const stripMarker = (state: HunkState): GutterMarker => gutterMarkers[CLASS[state]];

function build(state: Omit<ReviewState, "decos">, doc: Text, side: SideConfig): DecorationSet {
  const out: Range<Decoration>[] = [];
  state.spans.forEach((span, i) => {
    // The base side of a side-by-side view has no bar where the hunk adds lines: the filler
    // that aligns it counts the target side's bar already.
    if (span.empty && !side.bar) return;
    const hunk = { state: state.states[i], oldBaseLines: state.oldBase[i], baseDiffOpen: state.diffs.has(i) };
    out.push(
      Decoration.widget({
        widget: new HunkBar(i, side.headers[i], hunk, state.current === i, side.bar, side.m.hunk),
        block: true,
        side: -2,
      }).range(span.at),
    );
    const diff = state.diffs.get(i);
    // Between the bar and the deleted lines the unified view draws at the same place.
    if (diff && side.bar && hunk.state === "seenAgainstOldBase")
      out.push(Decoration.widget({ widget: new BaseDiffBlock(diff, side.m), block: true, side: -1.5 }).range(span.at));
    if (span.empty) return;
    const deco = Decoration.line({ class: CLASS[state.states[i]] });
    for (let pos = span.from; pos <= span.to;) {
      const line = doc.lineAt(pos);
      out.push(deco.range(line.from));
      pos = line.to + 1;
    }
  });
  return Decoration.set(out, true);
}

/**
 * Review decorations for one side. Positions are mapped through edits, so
 * decorations follow their content.
 */
export function reviewField(
  side: SideConfig,
  review: FileReview,
  diffs: ReadonlyMap<number, BaseDiff>,
): StateField<ReviewState> {
  return StateField.define<ReviewState>({
    create(state) {
      const doc = state.doc;
      const spans = side.lines.map(([start, len]): Span => {
        const at = start < doc.lines ? lineStart(doc, start) : doc.length;
        if (len === 0 || start >= doc.lines) return { from: 0, to: 0, empty: true, at };
        return { from: at, to: lineEnd(doc, start + len - 1), empty: false, at };
      });
      const base = { spans, states: review.hunks, oldBase: review.oldBaseLines, diffs, current: null };
      return { ...base, decos: build(base, doc, side) };
    },
    update(value, tr) {
      let next = value;
      if (tr.docChanged) {
        const map = (pos: number, assoc: -1 | 1) => tr.changes.mapPos(pos, assoc);
        next = {
          ...next,
          spans: next.spans.map((s) => ({
            ...s,
            at: map(s.at, 1),
            ...(s.empty ? {} : { from: map(s.from, -1), to: map(s.to, 1) }),
          })),
          decos: next.decos.map(tr.changes),
        };
      }
      let rebuild = false;
      for (const effect of tr.effects) {
        if (effect.is(setReview)) {
          next = { ...next, states: effect.value.hunks, oldBase: effect.value.oldBaseLines };
          rebuild = true;
        } else if (effect.is(setBaseDiffs)) {
          next = { ...next, diffs: effect.value };
          rebuild = true;
        } else if (effect.is(setCurrent)) {
          next = { ...next, current: effect.value };
          rebuild = true;
        }
      }
      return rebuild ? { ...next, decos: build(next, tr.state.doc, side) } : next;
    },
    provide: (f) => [
      EditorView.decorations.from(f, (v) => v.decos),
      // The 3px review-state strip: the leftmost gutter, one marker per changed line.
      gutter({
        class: "vet-strip",
        lineMarkerChange: (u) =>
          u.transactions.some((tr) => tr.effects.some((e) => e.is(setReview) || e.is(setCurrent))),
        lineMarker: (view, line) => {
          const state = view.state.field(f);
          const i = state.spans.findIndex((s) => !s.empty && s.from <= line.from && line.from <= s.to);
          return i < 0 ? null : gutterMarkers[CLASS[state.states[i]]];
        },
      }),
      // The bars start right of the gutters and stay there while the editor scrolls sideways.
      gutterWidth,
    ],
  });
}

/**
 * The unified view renders deleted lines as block widgets that carry no line
 * decorations, so they get the review-state classes of their hunk after each
 * update. A widget belongs to the hunk whose `at` is the widget's position.
 */
export function deletedChunkStyle(field: StateField<ReviewState>): Extension {
  const style = (view: EditorView) => {
    const state = view.state.field(field);
    const byPos = new Map(state.spans.map((s, i) => [s.at, i]));
    const baseLength = getOriginalDoc(view.state).length;
    const chunks = getChunks(view.state)?.chunks ?? [];
    for (const el of view.dom.querySelectorAll<HTMLElement>(".cm-deletedChunk")) {
      el.classList.remove(...Object.values(CLASS));
      const at = view.posAtDOM(el);
      const i = byPos.get(at);
      if (i !== undefined) el.classList.add(CLASS[state.states[i]]);
      const chunk = chunks.find((c) => c.fromB === at);
      el.classList.toggle("vet-base-tail", !!chunk && replacesBaseTail(chunk, baseLength));
    }
  };
  const plugin = ViewPlugin.define((view) => {
    // Widgets are created after plugin updates, so style them in the measure phase.
    const schedule = () => view.requestMeasure({ read: () => null, write: () => style(view), key: field });
    schedule();
    return {
      // Not on height-only updates: the measure phase sends those itself, and asking for
      // another measure from one restarts the measure loop until CodeMirror gives up.
      update: (u: ViewUpdate) => {
        if (u.docChanged || u.viewportChanged || u.transactions.some((tr) => tr.effects.some((e) => e.is(setReview))))
          schedule();
      },
    };
  });
  return [plugin, gutterWidth];
}
