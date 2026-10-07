// What the status footer says about the semantic layer: the diagnostics count and, per I6, that
// a file has no usable language server instead of silently showing nothing.

import type { Unavailable } from "../generated/Unavailable";

export interface SemanticSummary {
  /** Diagnostics of the files asked about so far; none before the first answer. */
  diagnostics?: number;
  /** Short footer text when a language server is missing, with the backend's message as `title`. */
  label?: string;
  title?: string;
  /** Why the focused file has no answers, if it has none. */
  missing?: Unavailable;
}

/**
 * `path` is the focused file tab's file; without it (the Changes tab) the diagnostics cover every
 * file asked about and no language server is named.
 */
export function summarize(
  counts: ReadonlyMap<string, number>,
  unavailable: ReadonlyMap<string, Unavailable>,
  path?: string,
): SemanticSummary {
  const paths = path === undefined ? [...counts.keys()] : counts.has(path) ? [path] : [];
  const diagnostics = paths.length ? paths.reduce((sum, p) => sum + (counts.get(p) ?? 0), 0) : undefined;
  // The Changes tab holds many files, so no one language's status applies there.
  const missing = path === undefined ? undefined : unavailable.get(path);
  if (!missing) return { diagnostics };
  return { diagnostics, label: missing.label, title: missing.message, missing };
}
