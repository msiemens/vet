import { Change, diff } from "@codemirror/merge";
import type { TextChange } from "../../../generated/TextChange";

// CodeMirror's documents have every line break as one "\n".
const asDocument = (text: string) => (text.includes("\r") ? text.replace(/\r\n?/g, "\n") : text);

/**
 * A `diffConfig.override` that answers the whole-document diff with the backend's
 * changes. Anything else, such as the slices around a local edit, differs from the
 * documents the changes are for and is diffed by CodeMirror.
 */
export function backendDiff(base: string, target: string, changes: readonly TextChange[]) {
  const [a, b] = [asDocument(base), asDocument(target)];
  let built: Change[] | undefined;
  return (docA: string, docB: string): readonly Change[] => {
    if (docA !== a || docB !== b) return diff(docA, docB);
    return (built ??= changes.map((c) => new Change(c.fromBase, c.toBase, c.fromTarget, c.toTarget)));
  };
}
