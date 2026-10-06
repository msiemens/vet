// Review-state decorations on every changed line (including the deleted lines of
// the unified view), hunk header bars and inline marks from the backend (SPEC §10).

import { StateEffect, StateField, type Extension, type Range, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, GutterMarker, ViewPlugin, gutter } from "@codemirror/view";
import type { Hunk } from "../../../generated/Hunk";
import type { Mark } from "../../../generated/Mark";
import type { MarkKind } from "../../../generated/MarkKind";
import type { HunkState } from "../../../generated/HunkState";
import { type BarActions, HunkBar, hunkHeader } from "./hunkBar";

export const setStates = StateEffect.define<HunkState[]>();
export const setCurrent = StateEffect.define<number | null>();

const CLASS: Record<HunkState, string> = { unseen: "vet-unseen", seen: "vet-seen", seenAgainstOldBase: "vet-old" };
const lineClass = (state: HunkState, current: boolean) => (current ? `${CLASS[state]} vet-current` : CLASS[state]);

const markDeco: Record<MarkKind, Decoration> = {
  symbol: Decoration.mark({ class: "vet-m-symbol" }),
  reachability: Decoration.mark({ class: "vet-m-reach" }),
  note: Decoration.mark({ class: "vet-m-note" }),
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
  marks: { from: number; to: number; kind: MarkKind }[];
  states: HunkState[];
  current: number | null;
  decos: DecorationSet;
}

export interface SideConfig {
  /** `[start, length]` of each hunk on this side, 0-based lines. */
  lines: [number, number][];
  headers: string[];
  /** Absent on the base side of a side-by-side view. */
  bar: BarActions | null;
}

export function sideConfig(
  hunks: Hunk[],
  side: "base" | "target",
  symbols: (string | undefined)[],
  bar: BarActions | null,
): SideConfig {
  return {
    lines: hunks.map((h) => (side === "base" ? [h.baseStart, h.baseLen] : [h.targetStart, h.targetLen])),
    headers: hunks.map((h, i) => hunkHeader(h, symbols[i])),
    bar,
  };
}

const gutterMarkers: Record<string, GutterMarker> = {};
for (const cls of Object.values(CLASS)) {
  gutterMarkers[cls] = new (class extends GutterMarker {
    elementClass = cls;
  })();
}

function build(state: Omit<ReviewState, "decos">, doc: Text, side: SideConfig): DecorationSet {
  const out: Range<Decoration>[] = [];
  state.spans.forEach((span, i) => {
    out.push(
      Decoration.widget({
        widget: new HunkBar(i, side.headers[i], state.states[i], side.bar),
        block: true,
        side: -2,
      }).range(span.at),
    );
    if (span.empty) return;
    const deco = Decoration.line({ class: lineClass(state.states[i], state.current === i) });
    for (let pos = span.from; pos <= span.to;) {
      const line = doc.lineAt(pos);
      out.push(deco.range(line.from));
      pos = line.to + 1;
    }
  });
  for (const m of state.marks) if (m.from < m.to) out.push(markDeco[m.kind].range(m.from, m.to));
  return Decoration.set(out, true);
}

/**
 * Review decorations for one side. Positions are mapped through edits, so
 * decorations follow their content.
 */
export function reviewField(side: SideConfig, marks: Mark[], states: HunkState[]): StateField<ReviewState> {
  return StateField.define<ReviewState>({
    create(state) {
      const doc = state.doc;
      const spans = side.lines.map(([start, len]): Span => {
        const at = start < doc.lines ? doc.line(start + 1).from : doc.length;
        if (len === 0 || start >= doc.lines) return { from: 0, to: 0, empty: true, at };
        return { from: at, to: doc.line(Math.min(start + len, doc.lines)).to, empty: false, at };
      });
      const placed = marks.flatMap((m) => {
        if (m.line >= doc.lines) return [];
        const line = doc.line(m.line + 1);
        return [{ from: Math.min(line.from + m.from, line.to), to: Math.min(line.from + m.to, line.to), kind: m.kind }];
      });
      const base = { spans, marks: placed, states, current: null };
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
          marks: next.marks.map((m) => ({ ...m, from: map(m.from, -1), to: map(m.to, 1) })),
          decos: next.decos.map(tr.changes),
        };
      }
      let rebuild = false;
      for (const effect of tr.effects) {
        if (effect.is(setStates)) {
          next = { ...next, states: effect.value };
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
          u.transactions.some((tr) => tr.effects.some((e) => e.is(setStates) || e.is(setCurrent))),
        lineMarker: (view, line) => {
          const state = view.state.field(f);
          const i = state.spans.findIndex((s) => !s.empty && s.from <= line.from && line.from <= s.to);
          return i < 0 ? null : gutterMarkers[CLASS[state.states[i]]];
        },
      }),
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
    // The strip of a deleted block reaches back over the gutters to the editor's left edge.
    view.dom.style.setProperty(
      "--vet-gutters",
      `${view.dom.querySelector<HTMLElement>(".cm-gutters")?.offsetWidth ?? 0}px`,
    );
    for (const el of view.dom.querySelectorAll<HTMLElement>(".cm-deletedChunk")) {
      el.classList.remove(...Object.values(CLASS), "vet-current");
      const i = byPos.get(view.posAtDOM(el));
      if (i !== undefined) el.classList.add(...lineClass(state.states[i], state.current === i).split(" "));
    }
  };
  return ViewPlugin.define((view) => {
    // Widgets are created after plugin updates, so style them in the measure phase.
    const schedule = () => view.requestMeasure({ read: () => null, write: () => style(view), key: field });
    schedule();
    return { update: schedule };
  });
}
