// What a row of the commit list shows of a commit, as the backend described it (SPEC I5: no
// progress or classification is worked out here).

import type { CommitUnit } from "../../generated/CommitUnit";
import { shortTime } from "../../lib/shortTime.ts";

export interface CommitRow {
  subject: string;
  shortId: string;
  author: string;
  when: string;
  merge: boolean;
  /** A merge that decided nothing beyond the automatic merge: muted, and there is nothing to review. */
  clean: boolean;
  /** A merge compared with its first parent, since there was no automatic merge to compare it with. */
  notRemerged: boolean;
  added: number;
  removed: number;
  /** The shares of the commit's own diff that are seen, and seen against another base, in percent. */
  seen: number;
  oldBase: number;
}

export function commitRow(unit: CommitUnit, now = Date.now()): CommitRow {
  const merge = unit.kind.type === "merge";
  const clean = unit.kind.type === "merge" && unit.kind.clean;
  return {
    subject: unit.commit.subject,
    shortId: unit.commit.shortId,
    author: unit.commit.author,
    when: shortTime(unit.commit.time, now, "detailed"),
    merge,
    clean,
    notRemerged: unit.kind.type === "merge" && !unit.kind.remerged,
    added: unit.delta.added,
    removed: unit.delta.removed,
    seen: unit.completion.seenPercent,
    oldBase: unit.completion.oldBasePercent,
  };
}
