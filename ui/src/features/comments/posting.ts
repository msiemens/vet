// Posting the viewer's comments, the same from the Comments tool window and the toolbar's popover:
// as one review with the verdict and summary of the Submit review form, or one after the other and
// then the staged vote.

import type { PullRequestInfo } from "../../generated/PullRequestInfo";
import type { Verdict } from "../../generated/Verdict";
import { notices } from "../../state/notices.svelte";
import { platform } from "../../state/platform.svelte";
import { commentsUi } from "./commentsUi.svelte";
import { postedNote, waitingNote } from "./pullRequest";

/** The verdict the form has chosen, else the platform's first. */
export const chosenVerdict = (info: PullRequestInfo): Verdict | undefined =>
  commentsUi.review.verdict ?? info.traits.verdicts.at(0);

/** Posts the outgoing comments; resolves whether all of them went through. What stopped it is told by the backend's notification. */
export async function sendComments(info: PullRequestInfo): Promise<boolean> {
  if (commentsUi.sending) return false;
  commentsUi.sending = true;
  try {
    if (info.traits.batchedReview) {
      const verdict = chosenVerdict(info);
      if (verdict === undefined) return false;
      const done = await platform.submit(verdict, commentsUi.review.body);
      if (done?.problem !== null) return false;
      commentsUi.sentReview();
      notices.show(done.skipped > 0 ? `Review submitted. ${waitingNote(done.skipped)}` : "Review submitted");
    } else {
      const done = await platform.post();
      if (done?.problem !== null) return false;
      notices.show(postedNote(done));
    }
    return true;
  } finally {
    commentsUi.sending = false;
  }
}
