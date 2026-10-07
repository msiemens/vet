// A read-only view of one file's text in the target snapshot (Jump to Source): a CodeMirror
// editor with syntax highlighting, line numbers and the semantic layer (hover, go to definition,
// diagnostics), or a small placeholder when the file has no text to show.

import { EditorState } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import type { NoTextReason } from "../../../generated/NoTextReason";
import type { TextContent } from "../../../generated/TextContent";
import type { TextSpan } from "../../../generated/TextSpan";
import { errorText } from "../../../lib/errorText";
import { iconSvg } from "../../../lib/ui/icons";
import { atRevision } from "../../../state/revision";
import { highlightExtension } from "./highlightExtension";
import { decodeHighlights } from "./highlights";
import { type SemanticHooks, revealSpan, semanticExtensions, showDiagnostics } from "./semantic";

const NO_TEXT: Record<NoTextReason, string> = {
  binary: "Binary file: there is no text to show.",
  unavailable: "The content of this file is not available: it is over the size limit, or it could not be read.",
  notAFile: "Not a regular file: a symlink or a submodule has no text.",
};

export class TextView {
  private view?: EditorView;
  private destroyed = false;
  /** The 0-based line to show once the text has arrived. */
  private line?: number;
  /** The span to show once the text has arrived. */
  private span?: TextSpan;

  /**
   * Fills `root` with the file's text as of `revision`. A reply of another revision is dropped:
   * the owner builds a new view when the manifest moves.
   */
  constructor(
    private root: HTMLElement,
    private path: string,
    revision: number,
    fetch: () => Promise<{ revision: number; content: TextContent }>,
    private hooks: SemanticHooks,
  ) {
    root.replaceChildren();
    root.classList.add("vet-text");
    fetch().then(
      (reply) => {
        const text = atRevision(revision, reply);
        if (this.destroyed || !text) return;
        if (text.content.type === "text") this.mountEditor(text.content.text, text.content.highlights);
        else this.placeholder(NO_TEXT[text.content.reason]);
      },
      (error: unknown) => {
        if (!this.destroyed) this.placeholder(errorText(error));
      },
    );
  }

  private mountEditor(text: string, highlights: number[]) {
    this.view = new EditorView({
      parent: this.root,
      doc: text,
      extensions: [
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
    const { from } = view.state.doc.line(Math.min(line + 1, view.state.doc.lines));
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
