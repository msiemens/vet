// The changeset switcher's menu, from the rows the backend listed.

import type { ChangesetKey } from "../../generated/ChangesetKey";
import type { ChangesetRow } from "../../generated/ChangesetRow";
import type { RowReview } from "../../generated/RowReview";
import type { MenuItem } from "../../lib/ui/menuItem.ts";
import { keyId } from "../../state/changesetKey.ts";
import { relativeTime } from "./relativeTime.ts";

function reviewText(review: RowReview): string | undefined {
  switch (review.state) {
    case "open":
      return `reviewing · v${review.version}`;
    case "finished":
      return review.complete ? "reviewed" : "review finished, unseen changes left";
    case "none":
      return undefined;
  }
}

/** What a row says besides its name: how far it is from the default branch, its age and its review. */
export function rowDetail(row: ChangesetRow, nowMs = Date.now()): string {
  const d = row.divergence;
  return [
    d && `↑${d.ahead} ↓${d.behind}`,
    row.lastCommit !== undefined && relativeTime(row.lastCommit, nowMs),
    row.checkedOut && "checked out",
    reviewText(row.review),
  ]
    .filter(Boolean)
    .join(" · ");
}

/** One item per row, its id the row's `keyId`; the one on view is checked. */
export function changesetItems(rows: ChangesetRow[], onView: ChangesetKey | undefined, nowMs = Date.now()): MenuItem[] {
  const current = onView && keyId(onView);
  return rows.map((row) => ({
    id: keyId(row.key),
    label: row.title,
    detail: rowDetail(row, nowMs) || undefined,
    checked: keyId(row.key) === current,
  }));
}
