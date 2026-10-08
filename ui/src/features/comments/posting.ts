// Posting the viewer's comments, the same from the Comments tool window and the toolbar's popover:
// as one review with the verdict and summary of the Submit review form, or one after the other.

import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import { platform } from "../../state/platform.svelte";
import { session } from "../../state/session.svelte";
import { waitingNote } from "./pullRequest";

/** The verdict the form has chosen, else the platform's first. */
export const chosenVerdict = (info: PullRequestInfo): string =>
  platform.review.verdict ?? info.traits.verdicts.at(0)?.id ?? "";

/** Posts the outgoing comments; resolves whether all of them went through. What stopped it is told by the backend's notification. */
export async function sendComments(info: PullRequestInfo): Promise<boolean> {
  if (platform.sending) return false;
  platform.sending = true;
  try {
    if (info.traits.batchedReview) {
      const done = await platform.submit(chosenVerdict(info), platform.review.body);
      if (done?.problem !== null) return false;
      [platform.review.open, platform.review.body] = [false, ""];
      session.notice = done.skipped > 0 ? `Review submitted. ${waitingNote(done.skipped)}` : "Review submitted";
    } else {
      const done = await platform.post();
      if (done?.problem !== null) return false;
      const posted = `${done.posted} ${done.posted === 1 ? "comment" : "comments"} posted`;
      session.notice = done.skipped > 0 ? `${posted}. ${waitingNote(done.skipped)}` : posted;
    }
    return true;
  } finally {
    platform.sending = false;
  }
}
