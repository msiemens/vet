// The words of the confirmation before a project is removed: what is asked and what would be lost.

import type { ProjectStorage } from "../generated/ProjectStorage.ts";
import { plural } from "../lib/format.ts";

/** What removing the project loses and the viewer may have meant to keep, one line each; none for nothing. */
export function lostItems(p: Pick<ProjectStorage, "openReviews" | "unpostedComments">): string[] {
  return [
    ...(p.openReviews > 0 ? [plural(p.openReviews, "open review")] : []),
    ...(p.unpostedComments > 0 ? [plural(p.unpostedComments, "unposted comment")] : []),
  ];
}

/** The question; with something to lose it leads into the list that follows it. */
export function removeQuestion(name: string, lost: readonly string[]): string {
  const question = `Do you want to remove project ${name}?`;
  return lost.length ? `${question} The following will be lost:` : question;
}
