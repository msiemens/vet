// What the status footer says about the semantic layer: the diagnostics count and, per I6, that
// a file has no usable language server instead of silently showing nothing.

import type { Unavailable } from "../generated/Unavailable";

export interface SemanticSummary {
  /** Diagnostics of the files asked about so far; none before the first answer. */
  diagnostics?: number;
  /** Short footer text when a language server is missing, with the backend's message as `title`. */
  label?: string;
  title?: string;
}

/**
 * `path` is the focused file tab's file; without it (the Changes tab) the summary covers every
 * file asked about.
 */
export function summarize(
  counts: ReadonlyMap<string, number>,
  unavailable: ReadonlyMap<string, Unavailable>,
  path?: string,
): SemanticSummary {
  const paths = path === undefined ? [...counts.keys()] : counts.has(path) ? [path] : [];
  const diagnostics = paths.length ? paths.reduce((sum, p) => sum + (counts.get(p) ?? 0), 0) : undefined;
  const missing = path === undefined ? [...unavailable.values()] : [unavailable.get(path)].filter((u) => !!u);
  const first = missing.at(0);
  if (!first) return { diagnostics };
  const files = missing.length > 1 ? ` · ${missing.length} files` : "";
  return { diagnostics, label: first.label + files, title: first.message };
}
