// The small toolbar at the top right of the current hunk's lines in the File view: mark seen and
// Comment. It lies over the code, so while the caret or the pointer is on code it would cover, it
// fades and lets the clicks through; where nothing lies under it, it stays solid.

import type { Extension, StateField } from "@codemirror/state";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { commands } from "../../../state/commands";
import { el, iconButton, reconfirmButton } from "./dom";
import type { ChangesSource, ChangesState } from "./fileChanges";
import { LABEL } from "./hunkBar";
import { lineStart } from "./positions";

/** Space kept between the toolbar and the editor's right edge. */
const RIGHT_MARGIN = 8;

interface Reading {
  /** Where the toolbar goes, in px of the editor's box; none when the hunk is out of view. */
  at?: { top: number; left: number; width: number; height: number };
  /** Code lies under the toolbar. */
  covers: boolean;
  /** The caret is on a line the toolbar lies over. */
  caretUnder: boolean;
}

export function hunkToolbar(source: ChangesSource, field: StateField<ChangesState>): Extension {
  return ViewPlugin.fromClass(
    class {
      dom = el("div", "surface vet-fltools");
      private hunk: number | null = null;
      private pointer?: { x: number; y: number };
      private rect?: DOMRect;
      private caretUnder = false;
      private covers = false;

      constructor(readonly view: EditorView) {
        view.dom.append(this.dom);
        view.scrollDOM.addEventListener("scroll", this.measure, { passive: true });
        view.dom.addEventListener("pointermove", this.onPointer, { passive: true });
        view.dom.addEventListener("pointerleave", this.onLeave, { passive: true });
        this.measure();
      }

      update(u: ViewUpdate) {
        const changed = u.startState.field(field) !== u.state.field(field);
        if (changed || u.docChanged || u.viewportChanged || u.geometryChanged || u.selectionSet) this.measure();
      }

      destroy() {
        this.view.scrollDOM.removeEventListener("scroll", this.measure);
        this.view.dom.removeEventListener("pointermove", this.onPointer);
        this.view.dom.removeEventListener("pointerleave", this.onLeave);
        this.dom.remove();
      }

      private measure = () => {
        this.view.requestMeasure({
          read: (view) => this.read(view),
          write: (reading) => this.write(reading),
          key: this,
        });
      };

      private onPointer = (e: PointerEvent) => {
        this.pointer = { x: e.clientX, y: e.clientY };
        this.fade();
      };

      private onLeave = () => {
        this.pointer = undefined;
        this.fade();
      };

      /** Faded and click-through while the caret or the pointer is on code the toolbar lies over. */
      private fade() {
        const r = this.rect;
        const pointerUnder =
          !!r &&
          !!this.pointer &&
          this.pointer.x >= r.left &&
          this.pointer.x <= r.right &&
          this.pointer.y >= r.top &&
          this.pointer.y <= r.bottom;
        this.dom.classList.toggle("vet-fltools-faded", this.covers && (this.caretUnder || pointerUnder));
      }

      private read(view: EditorView): Reading {
        const s = view.state.field(field);
        const hunk = s.current !== null && source.hunks[s.current].targetLen > 0 ? s.current : null;
        this.hunk = hunk;
        if (hunk === null) return { covers: false, caretUnder: false };
        // The buttons are drawn before they are measured.
        this.fill(hunk, s);
        this.dom.style.display = "";
        const h = source.hunks[hunk];
        const doc = view.state.doc;
        const box = view.dom.getBoundingClientRect();
        const scroller = view.scrollDOM.getBoundingClientRect();
        const first = view.lineBlockAt(lineStart(doc, h.targetStart));
        const last = view.lineBlockAt(lineStart(doc, h.targetStart + h.targetLen - 1));
        const top = view.documentTop - box.top + first.top;
        const bottom = view.documentTop - box.top + last.bottom;
        const visible = { top: scroller.top - box.top, bottom: scroller.bottom - box.top };
        const width = this.dom.offsetWidth;
        const height = this.dom.offsetHeight;
        if (bottom < visible.top || top > visible.bottom) return { covers: false, caretUnder: false };
        const placed = Math.min(Math.max(top, visible.top) + 2, bottom - height);
        const scrollbar = view.dom.clientWidth - view.scrollDOM.clientWidth;
        const left = view.dom.clientWidth - scrollbar - RIGHT_MARGIN - width;
        // The text of the lines under the toolbar reaches into it.
        let covers = false;
        for (let pos = first.from; pos <= last.to;) {
          const block = view.lineBlockAt(pos);
          const y = view.documentTop - box.top + block.top;
          if (y < placed + height && y + block.height > placed) {
            const end = view.coordsAtPos(block.to, -1);
            if (end && end.right - box.left > left - 8) covers = true;
          }
          pos = block.to + 1;
        }
        const caret = view.lineBlockAt(view.state.selection.main.head);
        const caretY = view.documentTop - box.top + caret.top;
        const caretUnder = caretY < placed + height && caretY + caret.height > placed;
        return { at: { top: placed, left, width, height }, covers, caretUnder };
      }

      private write(reading: Reading) {
        this.covers = reading.covers;
        this.caretUnder = reading.caretUnder;
        if (!reading.at) {
          this.dom.style.display = "none";
          this.rect = undefined;
          return;
        }
        this.dom.style.top = `${reading.at.top}px`;
        this.dom.style.left = `${reading.at.left}px`;
        this.rect = this.dom.getBoundingClientRect();
        this.fade();
      }

      private shown?: string;

      /** Draws the buttons for the hunk and its state, unless they are drawn. */
      private fill(hunk: number, s: ChangesState) {
        const state = s.states[hunk];
        const key = `${hunk}:${state}`;
        if (this.shown === key) return;
        this.shown = key;
        const { actions } = source;
        const buttons = [];
        if (state === "seenAgainstOldBase")
          buttons.push(
            reconfirmButton(commands.tooltip("review.reconfirm", "Re-confirm"), () => actions.toggleSeen(hunk)),
          );
        else if (state !== "informational")
          buttons.push(
            iconButton(
              commands.tooltip("review.toggleSeen", LABEL[state]),
              "check",
              () => actions.toggleSeen(hunk),
              "vet-icon-btn vet-review-action",
              13,
              2,
            ),
          );
        buttons.push(iconButton("Comment", "comments", () => actions.comment(hunk), "vet-icon-btn vet-review-action"));
        this.dom.replaceChildren(...buttons);
      }
    },
  );
}
