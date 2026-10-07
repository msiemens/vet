// The submit rules of a comment form, shared by the editor's thread cards and the Comments tool window.

/** A form posts only text with something in it, and one request at a time. */
export const canSubmit = (text: string, busy: boolean): boolean => !busy && text.trim() !== "";

/** Ctrl+Enter (Cmd+Enter on macOS) posts the form. */
export const isSubmitKey = (e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey">): boolean =>
  e.key === "Enter" && (e.ctrlKey || e.metaKey);
