// How the Comments tool window lists threads (SPEC §5.1, §10): detached ones first, then the
// ones whose anchor moved, the changeset's own, and the rest by file. Which group a thread is in
// follows from its anchor as the backend resolved it; nothing is decided here.

import type { LineLocation } from "../../generated/LineLocation";
import type { Thread } from "../../generated/Thread";
import { lineRange } from "../../lib/lineRange.ts";

export interface Groups {
  /** Ambiguous or stale: no place to show, offered "Attach to N" or "Discard". */
  detached: Thread[];
  /** Found by evidence, not where they were put: offered "Confirm". */
  moved: Thread[];
  changeset: Thread[];
  /** By path; a file's own threads first, then its lines in text order. */
  files: { path: string; threads: Thread[] }[];
}

const isDetached = (t: Thread) => t.anchor?.type === "stale" || t.anchor?.type === "ambiguous";

/** Threads are given in creation order, which each group keeps unless it says otherwise. */
export function groupThreads(threads: readonly Thread[]): Groups {
  const groups: Groups = { detached: [], moved: [], changeset: [], files: [] };
  const byPath = new Map<string, Thread[]>();
  for (const thread of threads) {
    if (isDetached(thread)) groups.detached.push(thread);
    else if (thread.anchor?.type === "inferred") groups.moved.push(thread);
    else if (thread.kind === "changeset") groups.changeset.push(thread);
    else {
      const path = thread.path ?? thread.location?.path ?? "";
      byPath.set(path, [...(byPath.get(path) ?? []), thread]);
    }
  }
  const start = (t: Thread) => (t.kind === "file" ? -1 : (t.location?.start ?? 0));
  for (const path of [...byPath.keys()].sort()) {
    const list = byPath.get(path) ?? [];
    groups.files.push({ path, threads: [...list].sort((a, b) => start(a) - start(b) || a.id - b.id) });
  }
  return groups;
}

/** The first line of the opening message, cut to `max` characters. */
export function preview(thread: Thread, max = 80): string {
  const first = thread.messages[0]?.body.split("\n")[0].trim() ?? "";
  return first.length > max ? `${first.slice(0, max - 1)}…` : first;
}

/** Where the thread is: its lines, or the file or changeset. A detached range is where it was recorded. */
export function placeLabel(thread: Thread): string {
  if (thread.kind === "changeset") return "Changeset";
  if (thread.kind === "file") return "File";
  const at = thread.location ?? thread.original;
  return at ? lineRange(at.start, at.len) : "";
}

/** Why a detached thread has no place. */
export function detachedReason(thread: Thread): string {
  const anchor = thread.anchor;
  if (anchor?.type === "ambiguous") return `${anchor.candidates.length} places match`;
  if (anchor?.type === "stale") return anchor.reason === "fileDeleted" ? "The file was deleted" : "The code changed";
  return "";
}

/** A candidate place of an ambiguous thread, as a menu entry. */
export function candidateLabel(at: LineLocation): string {
  return `${at.path}:${lineRange(at.start, at.len)}`;
}

/** The candidates an ambiguous thread can be attached to. */
export function candidates(thread: Thread): LineLocation[] {
  return thread.anchor?.type === "ambiguous" ? thread.anchor.candidates : [];
}
