// A read-only view of a CI log: line numbers, and the line where the job failed marked and scrolled
// to.

import { EditorState } from "@codemirror/state";
import { Decoration, EditorView, lineNumbers } from "@codemirror/view";
import { Scrollbar } from "../../../lib/ui/scrollbar";
import { lineStart } from "./positions";

const failure = Decoration.line({ class: "vet-log-failure" });

export class LogView {
  private view: EditorView;
  private line?: number;
  private scrollbar: Scrollbar;

  /** Fills `root` with `text`, with the 0-based line `failedAt` marked. */
  constructor(
    private root: HTMLElement,
    text: string,
    failedAt?: number,
  ) {
    root.replaceChildren();
    root.classList.add("vet-text");
    const doc = EditorState.create({ doc: text }).doc;
    this.line = failedAt !== undefined && failedAt < doc.lines ? failedAt : undefined;
    const marks =
      this.line === undefined ? Decoration.none : Decoration.set([failure.range(lineStart(doc, this.line))]);
    this.view = new EditorView({
      parent: root,
      doc,
      extensions: [lineNumbers(), EditorState.readOnly.of(true), EditorView.decorations.of(marks)],
    });
    this.scrollbar = new Scrollbar(this.view.scrollDOM);
    root.after(this.scrollbar.dom);
    this.goToFailure();
  }

  /** Scrolls the failure line to the middle of the view; there is no scrolling to do for a log without one. */
  goToFailure() {
    if (this.line === undefined) return;
    const at = lineStart(this.view.state.doc, this.line);
    this.view.dispatch({ selection: { anchor: at }, effects: EditorView.scrollIntoView(at, { y: "center" }) });
  }

  destroy() {
    this.scrollbar.destroy();
    this.view.destroy();
    this.root.classList.remove("vet-text");
    this.root.replaceChildren();
  }
}
