// What the header says about the review: what is shown (the scope chip) and what the changes are
// measured from, where that is worth saying (the base note).

import type { Base } from "../../generated/Base";
import { shortTime } from "../../lib/shortTime.ts";
import { type Range, pushesOf } from "../../state/timelineRange.ts";

/** What the scope chip needs to say what is shown. */
export interface ScopeFacts {
  /** The pushes shown; none for the whole changeset. */
  range?: Range;
  /** The newest version; none before a review has started. */
  latest?: number;
  /** The commit shown while the changeset is reviewed by commit. */
  commit?: { position: number; of: number; shortId?: string; subject?: string };
  /** The shown changes reach the working tree, which follows the files on disk. */
  live: boolean;
  /** Edits of the working tree wait to become a version. */
  pending: boolean;
}

export interface ScopeChip {
  label: string;
  tip: string;
  /** Not the whole changeset: it can be left. */
  narrowed: boolean;
  /** The dot after the label: the working tree is followed, hollow while edits wait to become a version. */
  dot?: "live" | "pending";
}

const WAITING = " Edits since then are being captured and become the next version.";

/** The chip for what is shown. */
export function scopeChip({ range, latest, commit, live, pending }: ScopeFacts): ScopeChip {
  if (commit) {
    const { position, of, shortId, subject } = commit;
    return {
      label: shortId ? `Commit ${shortId}` : `Commit ${position}`,
      tip: `${shortId ? `${shortId} ${subject ?? ""}`.trim() + ". " : ""}Commit ${position} of ${of}.`,
      narrowed: true,
    };
  }
  const dot = live ? (pending ? "pending" : "live") : undefined;
  const waiting = dot === "pending" ? WAITING : "";
  if (!range) {
    return {
      label: "All changes",
      tip: `Everything the changeset changes${latest === undefined ? "." : `, up to v${latest}.`}${waiting}`,
      narrowed: false,
      dot,
    };
  }
  const { first, last } = pushesOf(range);
  const against = range.from === 0 ? "the base" : `v${range.from}`;
  return {
    label: first === last ? `Push v${first}` : `Pushes v${first}–v${last}`,
    tip: `${first === last ? `What push v${first} changed` : `What pushes v${first} to v${last} changed`}, compared with ${against}.`,
    narrowed: true,
  };
}

/** The banner for a working tree whose HEAD left the base `shortId`. */
export const offBaseMessage = (shortId: string): string =>
  `HEAD no longer builds on the review's base (${shortId}). The changes shown include work that came from elsewhere, e.g. a rebase.`;

/** The kind of changeset the base belongs to. */
export type BaseOf = "workingTree" | "branch" | "pullRequest";

export interface BaseNote {
  text: string;
  tip: string;
  /** HEAD no longer builds on the base. */
  alert: boolean;
}

/**
 * What the header says of the base: only where the viewer would not expect it, a pull request
 * into a branch other than the default and a working tree whose HEAD left its base.
 */
export function baseNote(base: Base, of: BaseOf, nowMs: number): BaseNote | undefined {
  const alert = base.headOffBase;
  if (base.into === undefined && !alert) return undefined;
  const c = base.commit;
  const where = `${c.shortId} ${c.subject}, ${shortTime(c.time, nowMs, "detailed")}.`;
  const tips = {
    pullRequest: `Into ${base.into}. Branched off at ${where} Fixed for this pull request.`,
    branch: `Merge base with the default branch: ${where}`,
    workingTree: `HEAD when the workspace was opened: ${where} Agent commits on top do not move it.`,
  };
  return {
    text: of === "pullRequest" ? `into ${base.into}` : `since ${c.shortId}`,
    tip: tips[of] + (alert ? " HEAD no longer builds on this base." : ""),
    alert,
  };
}
