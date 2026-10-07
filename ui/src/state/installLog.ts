/** An install of a language server, as the UI started it. */
export interface InstallProgress {
  running: boolean;
  /** The backend's message for an install that did not work. */
  error?: string;
}

/** Keeps the newest `max` lines. */
export function appendLog(lines: readonly string[], line: string, max = 500): string[] {
  const next = [...lines, line];
  return next.length > max ? next.slice(next.length - max) : next;
}
