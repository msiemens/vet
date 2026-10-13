// What the status footer says about the semantic layer: whether a server has answered and, per I6,
// that a file has no usable language server instead of silently showing nothing.

import type { Unavailable } from "../generated/Unavailable";

export interface SemanticSummary {
  /** A server has answered for the file (any file, without one in focus). */
  answered: boolean;
  /** Short footer text when a language server is missing, with the backend's message as `title`. */
  label?: string;
  title?: string;
  /** Why the focused file has no answers, if it has none. */
  missing?: Unavailable;
}

/**
 * `path` is the focused file tab's file; without it (the Changes tab) every file asked about counts
 * and no language server is named.
 */
export function summarize(
  answered: ReadonlyMap<string, unknown>,
  unavailable: ReadonlyMap<string, Unavailable>,
  path?: string,
): SemanticSummary {
  const has = path === undefined ? answered.size > 0 : answered.has(path);
  // The Changes tab holds many files, so no one language's status applies there.
  const missing = path === undefined ? undefined : unavailable.get(path);
  if (!missing) return { answered: has };
  return { answered: has, label: missing.label, title: missing.message, missing };
}
