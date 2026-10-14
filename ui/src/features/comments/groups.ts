// How the Comments tool window lists threads (SPEC §5.1, §10): the ones with an edit to settle,
// the open ones and the resolved ones. A thread that is detached or moved says so in its row.

import type { LineLocation } from "../../generated/LineLocation";
import type { Thread } from "../../generated/Thread";
import { compareText } from "../../lib/format.ts";
import { lineRange } from "../../lib/lineRange.ts";

/** A comment of the thread was edited here and on the platform, and waits for the viewer to choose. */
export const hasConflict = (t: Thread) => t.messages.some((m) => m.conflict !== null);

/** How many of the thread's messages wait for Submit review. */
export const draftCount = (t: Thread) => (t.platform ? t.messages.filter((m) => m.state.type !== "posted").length : 0);

export interface Groups {
  /** Threads with an edit to settle, whatever their status. */
  conflicts: Thread[];
  open: Thread[];
  resolved: Thread[];
}

/** How many resolved threads are listed before "N more". */
export const RESOLVED_SHOWN = 3;

const start = (t: Thread) => (t.kind === "file" ? -1 : (t.location?.start ?? t.original?.start ?? 0));

/** The changeset's own threads first, then by file, a file's own thread before its lines in text order. */
export const inOrder = (a: Thread, b: Thread) =>
  compareText(a.path ?? "", b.path ?? "") || start(a) - start(b) || a.id - b.id;

/** Every thread is in exactly one group, each in the order of `inOrder`. */
export function groupThreads(threads: readonly Thread[]): Groups {
  const groups: Groups = { conflicts: [], open: [], resolved: [] };
  for (const thread of threads) {
    if (hasConflict(thread)) groups.conflicts.push(thread);
    else if (thread.status === "resolved") groups.resolved.push(thread);
    else groups.open.push(thread);
  }
  groups.conflicts.sort(inOrder);
  groups.open.sort(inOrder);
  groups.resolved.sort(inOrder);
  return groups;
}

/** The threads of `list` shown now: all, or the first few of a long list that is not `expanded`. */
export function shown<T>(list: readonly T[], expanded: boolean, limit = RESOLVED_SHOWN): { rows: T[]; more: number } {
  if (expanded || list.length <= limit) return { rows: [...list], more: 0 };
  return { rows: list.slice(0, limit), more: list.length - limit };
}

/** The file a thread is on, and `:12` to follow its name for one on lines; no place for the changeset's own. */
export function threadPlace(thread: Thread): { path: string; suffix: string } | undefined {
  if (thread.kind === "changeset" || thread.path === null) return undefined;
  const at = thread.location ?? thread.original;
  return { path: thread.path, suffix: thread.kind === "range" && at ? `:${at.start + 1}` : "" };
}

/** The first line of `text`, cut to `max` characters. */
export function firstLine(text: string, max = 80): string {
  const first = text.split("\n")[0].trim();
  return first.length > max ? `${first.slice(0, max - 1)}…` : first;
}

/** The first line of the opening message, cut to `max` characters. */
export const preview = (thread: Thread, max = 80): string => firstLine(thread.messages[0]?.body ?? "", max);

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
