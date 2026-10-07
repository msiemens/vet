// A read-only view of one file's text in the target snapshot (Jump to Source): a CodeMirror
// editor with syntax highlighting, line numbers and the semantic layer (hover, go to definition,
// diagnostics), or a small placeholder when the file has no text to show.

import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import type { NoTextReason } from "../../../generated/NoTextReason";
import type { TextSpan } from "../../../generated/TextSpan";
import { errorText } from "../../../lib/errorText";
import { decodeHighlights } from "../../../lib/highlights";
import { iconSvg } from "../../../lib/ui/icons";
import { type FileSource, StaleRevision } from "./fileSource";
import { highlightExtension } from "./highlightExtension";
import { lineStart } from "./positions";
import { type SemanticHooks, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";
import { softWrap, wrapsByDefault } from "./softWrap";

const NO_TEXT: Record<NoTextReason, string> = {
  binary: "Binary file: there is no text to show.",
  unavailable: "The content of this file is not available: it is over the size limit, or it could not be read.",
  notAFile: "Not a regular file: a symlink or a submodule has no text.",
};

export class TextView {
  private view?: EditorView;
  private wrapping = new Compartment();
  private destroyed = false;
  /** The 0-based line to show once the text has arrived. */
  private line?: number;
  /** The span to show once the text has arrived. */
  private span?: TextSpan;

  /**
   * Fills `root` with the file's text from `source`. A reply of another revision is dropped:
   * the owner builds a new view when the manifest moves.
   */
  constructor(
    private root: HTMLElement,
    private path: string,
    source: FileSource,
    private hooks: SemanticHooks,
    /** Soft wrap; absent: by the file's kind (`wrapsByDefault`). */
    private wrap = wrapsByDefault(path),
  ) {
    root.replaceChildren();
    root.classList.add("vet-text");
    source.text(path).then(
      (content) => {
        if (this.destroyed) return;
        if (content.type === "text") this.mountEditor(content.text, content.highlights);
        else this.placeholder(NO_TEXT[content.reason]);
      },
      (error: unknown) => {
        if (!this.destroyed && !(error instanceof StaleRevision)) this.placeholder(errorText(error));
      },
    );
  }

  private mountEditor(text: string, highlights: number[]) {
    this.view = new EditorView({
      parent: this.root,
      doc: text,
      extensions: [
        this.wrapping.of(softWrap(this.wrap)),
        lineNumbers(),
        highlightExtension(decodeHighlights(highlights)),
        // Read-only but focusable, so the caret and Mod+B work.
        EditorState.readOnly.of(true),
        semanticExtensions(this.path, "target", this.hooks),
      ],
    });
    if (this.span) this.goToSpan(this.span);
    else if (this.line !== undefined) this.goToLine(this.line);
    this.refreshDiagnostics();
  }

  wrapped(): boolean {
    return this.wrap;
  }

  /** Wraps long lines, or scrolls them sideways. */
  setWrap(on: boolean) {
    this.wrap = on;
    this.view?.dispatch({ effects: this.wrapping.reconfigure(softWrap(on)) });
  }

  /** Asks for the file's diagnostics again (the first time, or after the servers changed). */
  refreshDiagnostics() {
    const view = this.view;
    if (!view) return;
    void this.hooks.diagnostics(this.path).then((list) => {
      if (!this.destroyed) showDiagnostics(view, list);
    });
  }

  /** Puts the caret on a 0-based line and centres it. */
  goToLine(line: number) {
    this.line = line;
    const view = this.view;
    if (!view) return;
    const from = lineStart(view.state.doc, line);
    view.dispatch({ selection: { anchor: from }, effects: EditorView.scrollIntoView(from, { y: "center" }) });
  }

  /** Selects and marks a span, and centres it. */
  goToSpan(span: TextSpan) {
    this.span = span;
    if (this.view) revealSpan(this.view, span, true);
  }

  private placeholder(message: string) {
    const el = document.createElement("div");
    el.className = "vet-notext";
    el.innerHTML = iconSvg("file", 16, 1.6);
    el.append(message);
    this.root.replaceChildren(el);
  }

  destroy() {
    this.destroyed = true;
    this.view?.destroy();
    this.root.classList.remove("vet-text");
    this.root.replaceChildren();
  }
}
