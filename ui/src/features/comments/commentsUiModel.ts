// The Comments tool window's and the outgoing popover's own state, and the intents that change it:
// the pull request's data stays in `state/platform.svelte.ts` and nothing of it lives here. Plain, so
// the transitions and the subscriptions are tested without a component (`commentsUi.test.ts`); the
// reactive wrapper is `commentsUi.svelte.ts`.

import type { Verdict } from "../../generated/Verdict";
import type { Opening } from "../../state/openedProtocol";

/** The tab the Comments tool window shows. */
export type CommentsTab = "review" | "conversation";

/** The Submit review form. */
export interface ReviewForm {
  open: boolean;
  verdict: Verdict | undefined;
  body: string;
}

/** The comment on the changeset as a whole being written in the Comments tool window. */
export interface ChangesetComment {
  composing: boolean;
  body: string;
}

/** Everything the Comments UI keeps of its own. */
export interface CommentsUiState {
  /** The toolbar's list of the outgoing comments and the vote, with the button that posts them, is open. */
  outgoingOpen: boolean;
  /** While that popover is open it lists the votes to pick from instead. */
  votePicking: boolean;
  /** The Submit review form, kept so that switching tool windows does not empty it. */
  review: ReviewForm;
  /** The comment on the changeset as a whole being written in the Comments tool window. */
  changesetComment: ChangesetComment;
  /** The tab the Comments tool window shows. */
  commentsTab: CommentsTab;
  /** The viewer's comments are being posted. */
  sending: boolean;
}

/** The Comments UI with nothing open, composing or posting. */
export function initialCommentsUi(): CommentsUiState {
  return {
    outgoingOpen: false,
    votePicking: false,
    review: { open: false, verdict: undefined, body: "" },
    changesetComment: { composing: false, body: "" },
    commentsTab: "review",
    sending: false,
  };
}

/** Shows the outgoing comments and the vote; `picking` lists the votes to pick from instead. */
export function openOutgoing(state: CommentsUiState, picking: boolean): void {
  state.votePicking = picking;
  state.outgoingOpen = true;
}

/** Hides the outgoing popover; the vote picker goes with it. */
export function closeOutgoing(state: CommentsUiState): void {
  state.outgoingOpen = false;
  state.votePicking = false;
}

/** Shows the tab the Comments tool window shows. */
export function showTab(state: CommentsUiState, tab: CommentsTab): void {
  state.commentsTab = tab;
}

/** Opens the form for a comment on the changeset as a whole. */
export function composeChangesetComment(state: CommentsUiState): void {
  state.changesetComment.composing = true;
}

/** Closes the changeset comment form, keeping what was written. */
export function cancelChangesetComment(state: CommentsUiState): void {
  state.changesetComment.composing = false;
}

/** The changeset comment was posted, or the pull request changed: the form closes and starts empty. */
export function clearChangesetComment(state: CommentsUiState): void {
  state.changesetComment.composing = false;
  state.changesetComment.body = "";
}

/** The review was submitted: its form closes and starts empty, the verdict staying. */
export function clearReviewForm(state: CommentsUiState): void {
  state.review.open = false;
  state.review.body = "";
}

/** Shows or hides the Submit review form. */
export function toggleReviewForm(state: CommentsUiState): void {
  state.review.open = !state.review.open;
}

/** Hides the Submit review form, keeping its verdict and summary. */
export function closeReviewForm(state: CommentsUiState): void {
  state.review.open = false;
}

/** Where the Comments UI hears of a changed pull request or a closed project. */
export interface CommentsUpstream {
  /** Calls `listener` as a changeset is put on view. */
  onOpening: (listener: (opening: Opening) => void) => void;
  /** Calls `listener` when the project is closed. */
  onClose: (listener: () => void) => void;
}

/**
 * Keeps the Comments UI's own state right: another pull request starts with no comment being written,
 * and a closed project has no popover up.
 */
export function followCommentsUi(upstream: CommentsUpstream, state: CommentsUiState): void {
  upstream.onOpening((opening) => {
    if (opening.type === "opened") clearChangesetComment(state);
  });
  upstream.onClose(() => closeOutgoing(state));
}
