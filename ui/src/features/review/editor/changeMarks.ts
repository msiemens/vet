// Change marks in a file's text view: a gutter strip beside the lines the changeset added or changed
// and a notch where lines were removed, coloured by the hunk's review state like the diff's strip.
// Pressing a mark hands the hunk and the mark's element to the owner, which shows the hunk's actions.

import { type Extension, StateEffect, StateField } from "@codemirror/state";
import { GutterMarker, gutter } from "@codemirror/view";
import type { Hunk } from "../../../generated/Hunk";
import type { HunkState } from "../../../generated/HunkState";
import { type LineMark, type MarkKind, markedLines } from "./markedLines";

export const setMarkStates = StateEffect.define<HunkState[]>();

interface Marks {
  lines: Map<number, LineMark>;
  states: HunkState[];
}

class Mark extends GutterMarker {
  constructor(
    readonly kind: MarkKind,
    readonly state: HunkState,
  ) {
    super();
  }

  eq(other: Mark): boolean {
    return other.kind === this.kind && other.state === this.state;
  }

  toDOM(): HTMLElement {
    const mark = document.createElement("div");
    mark.className = `vet-mark vet-mark-${this.kind} vet-mark-${this.state}`;
    return mark;
  }
}

/** The marks of a text of the hunks' target side, in a gutter left of the line numbers. */
export function changeMarks(
  hunks: readonly Hunk[],
  states: HunkState[],
  /** A mark was pressed: its hunk, and the element of the mark to place the hunk's actions at. */
  open: (hunk: number, mark: HTMLElement) => void,
): Extension {
  const field = StateField.define<Marks>({
    create: (state) => ({ lines: markedLines(hunks, state.doc.lines), states }),
    update(marks, tr) {
      for (const e of tr.effects) if (e.is(setMarkStates)) return { ...marks, states: e.value };
      return marks;
    },
  });
  return [
    field,
    gutter({
      class: "vet-marks",
      lineMarkerChange: (u) => u.transactions.some((tr) => tr.effects.some((e) => e.is(setMarkStates))),
      lineMarker(view, line) {
        const marks = view.state.field(field);
        const mark = marks.lines.get(view.state.doc.lineAt(line.from).number - 1);
        return mark ? new Mark(mark.kind, marks.states[mark.hunk]) : null;
      },
      domEventHandlers: {
        mousedown(view, line, event) {
          const mark = view.state.field(field).lines.get(view.state.doc.lineAt(line.from).number - 1);
          const target =
            event.target instanceof Element ? event.target.closest<HTMLElement>(".cm-gutterElement") : null;
          if (!mark || !target) return false;
          open(mark.hunk, target);
          return true;
        },
      },
    }),
  ];
}
