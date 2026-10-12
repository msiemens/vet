// The changes of a file drawn in its text (the File view): changed lines tinted by their hunk's
// review state with the changed words of modified lines, the review strip, a wedge where lines were
// removed and the peek of those lines, and the hunk the caret is in tinted blue over all of that,
// with its line numbers a stronger blue. The layout comes from `fileLayout`; this module keeps it
// with the review states and draws it.

import { type Extension, type Range, RangeSet, StateEffect, StateField, type Text } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  GutterMarker,
  type ViewUpdate,
  WidgetType,
  gutter,
  lineNumberMarkers,
  lineNumbers,
} from "@codemirror/view";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import type { TextChange } from "../../../generated/TextChange";
import { commands } from "../../../state/commands";
import { el, iconButton } from "./dom";
import { type FileLayout, type Tint, fileLayout, hunkAtLine, removedLines, tintOf, wedgeAt } from "./fileLayout";
import { LABEL } from "./hunkBar";
import { lineOf } from "./positions";
import { stripMarker } from "./reviewState";

/** What the owner does for a hunk of the file, by the hunk's index. */
export interface HunkActions {
  toggleSeen(hunk: number): void;
  toggleFlag(hunk: number): void;
  comment(hunk: number): void;
}

export const setStates = StateEffect.define<{ states: HunkState[]; flags: boolean[] }>();
/** Makes a hunk the current one whatever the caret is on (navigation, the wedge's click). */
export const setCurrentHunk = StateEffect.define<number | null>();
/** Shows or hides the lines a removal took. */
export const setPeek = StateEffect.define<{ hunk: number; open: boolean }>();

/** What a view of a file's changes is made from. */
export interface ChangesSource {
  hunks: readonly Hunk[];
  changes: readonly TextChange[];
  /** The base text, whose lines a removal's peek shows. */
  base: Text;
  actions: HunkActions;
  /** The change card of a changed line was asked for. */
  card(view: EditorView, line: number): void;
}

export interface ChangesState {
  layout: FileLayout;
  states: HunkState[];
  /** Per hunk, whether it is flagged for later review. */
  flags: boolean[];
  /** The hunk the caret is in, or that navigation or a wedge chose last. */
  current: number | null;
  /** The removals whose lines are shown. */
  peeks: ReadonlySet<number>;
  decos: DecorationSet;
  /** The current hunk's lines and their line numbers. */
  currentLines: DecorationSet;
  currentNumbers: RangeSet<GutterMarker>;
}

const lineDecos: Record<Tint, Decoration> = {
  full: Decoration.line({ class: "vet-fl vet-fl-full" }),
  faint: Decoration.line({ class: "vet-fl vet-fl-faint" }),
};
const wordDecos: Record<Tint, Decoration> = {
  full: Decoration.mark({ class: "vet-fw vet-fw-full" }),
  faint: Decoration.mark({ class: "vet-fw vet-fw-faint" }),
};

const currentHunkLine = Decoration.line({ class: "vet-current-hunk" });
const currentHunkNumber = new (class extends GutterMarker {
  elementClass = "vet-current-hunk";
})();

/** The current hunk's changed lines (a removal has none), as line decorations and line number markers. */
function currentMarks(
  layout: FileLayout,
  current: number | null,
  doc: Text,
): Pick<ChangesState, "currentLines" | "currentNumbers"> {
  const starts: number[] = [];
  if (current !== null)
    for (const [line, hunk] of layout.lineHunk) if (hunk === current) starts.push(doc.line(line + 1).from);
  starts.sort((a, b) => a - b);
  return {
    currentLines: Decoration.set(starts.map((from) => currentHunkLine.range(from))),
    currentNumbers: RangeSet.of(starts.map((from) => currentHunkNumber.range(from))),
  };
}

/** What the peek's button does to a removal in each state. */
const PEEK_LABEL: Record<HunkState, string> = {
  unseen: "Mark seen",
  seen: "Mark unseen",
  seenAgainstOldBase: "Re-confirm",
  informational: "Nothing left to review",
  unapplied: "Mark seen",
};

/** The lines a removal took, shown between the lines it sat between. */
class Peek extends WidgetType {
  constructor(
    readonly hunk: number,
    readonly state: HunkState,
    readonly lines: { number: number; text: string }[],
    readonly actions: HunkActions,
  ) {
    super();
  }

  eq(other: Peek): boolean {
    return other.hunk === this.hunk && other.state === this.state && other.lines === this.lines;
  }

  toDOM(view: EditorView): HTMLElement {
    const root = el("div", `vet-peek vet-peek-${this.state}`);
    const head = el("div", "vet-peek-head");
    const mark = el("button", "ctl vet-review-action", PEEK_LABEL[this.state]);
    mark.type = "button";
    mark.title = commands.tooltip("review.toggleSeen", LABEL[this.state]);
    mark.addEventListener("click", () => this.actions.toggleSeen(this.hunk));
    head.append(
      el("span", "vet-peek-count", `−${this.lines.length}`),
      el("span", "grow"),
      ...(this.state === "informational" ? [] : [mark]),
      iconButton(
        "Close",
        "close",
        () => view.dispatch({ effects: setPeek.of({ hunk: this.hunk, open: false }) }),
        "vet-icon-btn",
        12,
      ),
    );
    const body = el("div", "vet-peek-lines");
    for (const line of this.lines) {
      const row = el("div", "vet-peek-line");
      row.append(el("span", "vet-peek-no", String(line.number)), el("span", "vet-peek-text", line.text));
      body.append(row);
    }
    root.append(head, body);
    return root;
  }

  ignoreEvent(): boolean {
    return true;
  }
}

/** The lines of each removal, kept so that a redrawn peek is the same widget. */
const removed = new WeakMap<Hunk, { number: number; text: string }[]>();

function build(source: ChangesSource, s: Pick<ChangesState, "layout" | "states" | "peeks">, doc: Text): DecorationSet {
  const out: Range<Decoration>[] = [];
  for (const [line, hunk] of s.layout.lineHunk)
    out.push(lineDecos[tintOf(s.states[hunk])].range(doc.line(line + 1).from));
  for (const w of s.layout.words) out.push(wordDecos[tintOf(s.states[w.hunk])].range(w.from, w.to));
  for (const d of s.layout.deletions) {
    if (!s.peeks.has(d.hunk)) continue;
    const hunk = source.hunks[d.hunk];
    let lines = removed.get(hunk);
    if (!lines) removed.set(hunk, (lines = removedLines(hunk, source.base)));
    const widget = new Peek(d.hunk, s.states[d.hunk], lines, source.actions);
    const below = d.line >= doc.lines;
    out.push(
      Decoration.widget({ widget, block: true, side: below ? 1 : -1 }).range(
        below ? doc.length : doc.line(d.line + 1).from,
      ),
    );
  }
  return Decoration.set(out, true);
}

/** The state of a file's changes in its text: the layout, the review states, the current hunk and the open peeks. */
export function changesField(source: ChangesSource, states: HunkState[], flags: boolean[]): StateField<ChangesState> {
  return StateField.define<ChangesState>({
    create(state) {
      const layout = fileLayout(source.hunks, source.changes, state.doc);
      const base = { layout, states, flags, current: null, peeks: new Set<number>() };
      return { ...base, decos: build(source, base, state.doc), ...currentMarks(layout, null, state.doc) };
    },
    update(value, tr) {
      let next = value;
      let rebuild = false;
      let explicit = false;
      for (const effect of tr.effects) {
        if (effect.is(setStates)) {
          next = { ...next, ...effect.value };
          rebuild = true;
        } else if (effect.is(setCurrentHunk)) {
          next = { ...next, current: effect.value };
          explicit = true;
        } else if (effect.is(setPeek)) {
          const peeks = new Set(next.peeks);
          if (effect.value.open) peeks.add(effect.value.hunk);
          else peeks.delete(effect.value.hunk);
          next = { ...next, peeks };
          rebuild = true;
        }
      }
      if (!explicit && tr.selection) {
        const current = hunkAtLine(next.layout, lineOf(tr.state.doc, tr.selection.main.head)) ?? null;
        if (current !== next.current) next = { ...next, current };
      }
      if (next.current !== value.current) next = { ...next, ...currentMarks(next.layout, next.current, tr.state.doc) };
      return rebuild ? { ...next, decos: build(source, next, tr.state.doc) } : next;
    },
    provide: (f) => [
      EditorView.decorations.from(f, (s) => s.decos),
      EditorView.decorations.from(f, (s) => s.currentLines),
      lineNumberMarkers.from(f, (s) => s.currentNumbers),
    ],
  });
}

/** The wedge where lines were removed. */
class Wedge extends GutterMarker {
  constructor(
    readonly at: "above" | "below",
    readonly state: HunkState,
  ) {
    super();
  }

  eq(other: Wedge): boolean {
    return other.at === this.at && other.state === this.state;
  }

  toDOM(): HTMLElement {
    const edge = el("div", "vet-edge");
    edge.append(el("span", `vet-wedge vet-wedge-${this.at} vet-wedge-${this.state}`));
    return edge;
  }
}

/**
 * The extensions that draw a file's changes: line numbers (a press on one of a changed line asks for
 * its card), the gutter of wedges, the review strip and the tints. `field` is the file's `changesField`.
 */
export function fileChanges(source: ChangesSource, field: StateField<ChangesState>): Extension[] {
  const changed = (line: number, view: EditorView) => view.state.field(field).layout.lineHunk.has(line);
  const stateChanged = (u: ViewUpdate) => u.startState.field(field) !== u.state.field(field);
  return [
    field,
    lineNumbers({
      domEventHandlers: {
        mousedown(view, block) {
          const line = lineOf(view.state.doc, block.from);
          if (!changed(line, view)) return false;
          source.card(view, line);
          return true;
        },
      },
    }),
    gutter({
      class: "vet-edges",
      lineMarkerChange: stateChanged,
      lineMarker(view, block) {
        const s = view.state.field(field);
        const doc = view.state.doc;
        const line = lineOf(doc, block.from);
        const removal = s.layout.deletions.find((d) => wedgeAt(d, doc.lines).line === line);
        return removal ? new Wedge(wedgeAt(removal, doc.lines).at, s.states[removal.hunk]) : null;
      },
      domEventHandlers: {
        mousedown(view, block) {
          const doc = view.state.doc;
          const line = lineOf(doc, block.from);
          const removal = view.state.field(field).layout.deletions.find((d) => wedgeAt(d, doc.lines).line === line);
          if (!removal) return false;
          const open = !view.state.field(field).peeks.has(removal.hunk);
          view.dispatch({ effects: [setPeek.of({ hunk: removal.hunk, open }), setCurrentHunk.of(removal.hunk)] });
          return true;
        },
      },
    }),
    gutter({
      class: "vet-strip",
      lineMarkerChange: stateChanged,
      lineMarker(view, block) {
        const s = view.state.field(field);
        const hunk = s.layout.lineHunk.get(lineOf(view.state.doc, block.from));
        return hunk === undefined ? null : stripMarker(s.states[hunk]);
      },
      domEventHandlers: {
        mousedown(view, block) {
          const line = lineOf(view.state.doc, block.from);
          if (!changed(line, view)) return false;
          source.card(view, line);
          return true;
        },
      },
    }),
  ];
}
