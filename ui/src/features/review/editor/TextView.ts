// A read-only view of one file's text, for files the changeset does not change: a CodeMirror
// editor with syntax highlighting and line numbers, or a small placeholder when the file has no
// text to show.

import { EditorState } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import type { NoTextReason } from "../../../generated/NoTextReason";
import type { TextContent } from "../../../generated/TextContent";
import { iconSvg } from "../../../lib/ui/icons";
import { syntax } from "./language";

const NO_TEXT: Record<NoTextReason, string> = {
  binary: "Binary file: there is no text to show.",
  unavailable: "The content of this file is not available: it is over the size limit, or it could not be read.",
  notAFile: "Not a regular file: a symlink or a submodule has no text.",
};

export class TextView {
  private view?: EditorView;
  private destroyed = false;

  /**
   * Fills `root` with the file's text as of `revision`. A reply of another revision is dropped:
   * the owner builds a new view when the manifest moves.
   */
  constructor(
    private root: HTMLElement,
    path: string,
    revision: number,
    fetch: () => Promise<{ revision: number; content: TextContent }>,
  ) {
    root.replaceChildren();
    root.classList.add("vet-text");
    fetch().then(
      (reply) => {
        if (this.destroyed || reply.revision !== revision) return;
        if (reply.content.type === "text") this.mountEditor(path, reply.content.text);
        else this.placeholder(NO_TEXT[reply.content.reason]);
      },
      (error: unknown) => {
        if (!this.destroyed) this.placeholder(error instanceof Error ? error.message : String(error));
      },
    );
  }

  private mountEditor(path: string, text: string) {
    this.view = new EditorView({
      parent: this.root,
      doc: text,
      extensions: [lineNumbers(), syntax(path), EditorState.readOnly.of(true), EditorView.editable.of(false)],
    });
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
