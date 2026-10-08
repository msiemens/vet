// Soft wrap of the editors: whether a file's lines wrap, and the extension that does it.

import type { Extension } from "@codemirror/state";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { basename } from "../../../lib/paths.ts";
import type { SoftWrap } from "../../../lib/softWrap.ts";

/** Prose is read, not scanned: Markdown, reStructuredText and plain text. */
const PROSE_EXTENSIONS = new Set(["md", "markdown", "mdown", "mkd", "mkdn", "rst", "rest", "txt", "text"]);
/** Plain text files that carry no extension. */
const PROSE_NAMES = new Set([
  "readme",
  "license",
  "licence",
  "copying",
  "notice",
  "authors",
  "contributors",
  "changelog",
  "changes",
  "news",
  "todo",
]);

function isProse(path: string): boolean {
  const name = basename(path).toLowerCase();
  const dot = name.lastIndexOf(".");
  return dot > 0 ? PROSE_EXTENSIONS.has(name.slice(dot + 1)) : PROSE_NAMES.has(name);
}

/** Whether the file at `path` wraps its lines under the viewer's setting. */
export function wraps(path: string, setting: SoftWrap): boolean {
  return setting === "all" || (setting === "prose" && isProse(path));
}

/** The extension of an editor that wraps long lines, or none for one that scrolls them sideways. */
export const softWrap = (on: boolean): Extension => (on ? EditorView.lineWrapping : []);

/**
 * Calls `changed` when the editor's content height changed, which CodeMirror works out in its
 * measure phase: with wrapping the height depends on the width of the text, which only the DOM
 * knows. Reads nothing and asks for no measure, so it is safe in a height-only update.
 */
export const reportHeight = (changed: () => void): Extension =>
  ViewPlugin.define(() => ({
    update(update: ViewUpdate) {
      if (update.heightChanged) changed();
    },
  }));
