// The platform's ways to close a review as the UI gives them, and how one is named in a menu or a
// form: the backend sends the verdict shaped (`PlatformTraits.verdicts`), the words are here.

import type { Verdict } from "../generated/Verdict";

const LABELS: Record<Exclude<Verdict["type"], "other">, string> = {
  comment: "Comment",
  approve: "Approve",
  requestChanges: "Request changes",
  approveWithSuggestions: "Approve with suggestions",
  waitForAuthor: "Wait for author",
  reject: "Reject",
};

/** What a verdict reads as. One Vet does not know the spelling of reads as what it was spelled. */
export const verdictLabel = (verdict: Verdict): string =>
  verdict.type === "other" ? verdict.value : LABELS[verdict.type];

/** What names the verdict among a platform's, enough to find it again with `verdictOf`. */
export const verdictId = (verdict: Verdict): string => (verdict.type === "other" ? verdict.value : verdict.type);

/** The verdict an id of `verdictId` names; none other than a platform's can be used. */
export const verdictOf = (id: string): Verdict | undefined => {
  switch (id) {
    case "comment":
      return { type: "comment" };
    case "approve":
      return { type: "approve" };
    case "requestChanges":
      return { type: "requestChanges" };
    case "approveWithSuggestions":
      return { type: "approveWithSuggestions" };
    case "waitForAuthor":
      return { type: "waitForAuthor" };
    case "reject":
      return { type: "reject" };
    default:
      return undefined;
  }
};
