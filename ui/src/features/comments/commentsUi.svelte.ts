// The Comments tool window's and the outgoing popover's own state, kept reactive: the flags the
// components bind and the intents that change them (`commentsUiModel.ts` holds the plain transitions).
// The pull request's data is `state/platform.svelte.ts`'s.

import { session } from "../../state/session.svelte";
import {
  type CommentsTab,
  type CommentsUiState,
  clearReviewForm,
  closeOutgoing as closeOutgoingState,
  closeReviewForm as closeReviewFormState,
  followCommentsUi,
  initialCommentsUi,
  openOutgoing as openOutgoingState,
  showTab as showTabState,
  toggleReviewForm as toggleReviewFormState,
} from "./commentsUiModel";

class CommentsUi {
  private state = $state(initialCommentsUi());

  /** The toolbar's list of the outgoing comments and the vote, with the button that posts them, is open. */
  get outgoingOpen(): boolean {
    return this.state.outgoingOpen;
  }
  set outgoingOpen(open: boolean) {
    // The popover closes itself (a click outside, Escape, a resize) through this binding, so the
    // vote picker goes with it here rather than in the component's effect.
    if (open) openOutgoingState(this.state, this.state.votePicking);
    else closeOutgoingState(this.state);
  }

  /** While that popover is open it lists the votes to pick from instead. */
  get votePicking(): boolean {
    return this.state.votePicking;
  }

  /** The Submit review form, kept so that switching tool windows does not empty it. */
  get review(): CommentsUiState["review"] {
    return this.state.review;
  }

  /** The tab the Comments tool window shows. */
  get commentsTab(): CommentsTab {
    return this.state.commentsTab;
  }

  /** The viewer's comments are being posted. */
  get sending(): boolean {
    return this.state.sending;
  }
  set sending(sending: boolean) {
    this.state.sending = sending;
  }

  constructor() {
    followCommentsUi({ onClose: (listener) => session.onClose(listener) }, this.state);
  }

  /** Shows the outgoing comments and the vote; `picking` lists the votes to pick from instead. */
  openOutgoing(picking: boolean) {
    openOutgoingState(this.state, picking);
  }

  /** Hides the outgoing popover, and the vote picker with it. */
  closeOutgoing() {
    closeOutgoingState(this.state);
  }

  /** Shows the tab the Comments tool window shows. */
  showTab(tab: CommentsTab) {
    showTabState(this.state, tab);
  }

  /** Shows the Comments tool window's Conversation tab. */
  showConversation() {
    showTabState(this.state, "conversation");
  }

  /** The review was submitted: its form closes and starts empty, the verdict staying. */
  sentReview() {
    clearReviewForm(this.state);
  }

  /** Shows or hides the Submit review form. */
  toggleReviewForm() {
    toggleReviewFormState(this.state);
  }

  /** Hides the Submit review form, keeping its verdict and summary. */
  closeReviewForm() {
    closeReviewFormState(this.state);
  }
}

export const commentsUi = new CommentsUi();
export type { CommentsTab } from "./commentsUiModel";
