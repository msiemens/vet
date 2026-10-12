// The submit rules of a comment form, shared by the editor's thread cards and the Comments tool window.

/** A form posts only text with something in it, and one request at a time. */
export const canSubmit = (text: string, busy: boolean): boolean => !busy && text.trim() !== "";

/**
 * How a reply field shows: one row until it is clicked or holds text, then room for a longer
 * answer; its button is there only once there is text to send.
 */
export function replyShape(text: string, focused: boolean): { rows: number; send: boolean } {
  return { rows: focused || text !== "" ? 3 : 1, send: text.trim() !== "" };
}

/** The key that posts the form, as a binding spec: Ctrl+Enter, Cmd+Enter on macOS. */
export const SUBMIT_KEY = "Mod-Enter";

/** Ctrl+Enter (Cmd+Enter on macOS) posts the form. */
export const isSubmitKey = (e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey">): boolean =>
  e.key === "Enter" && (e.ctrlKey || e.metaKey);
