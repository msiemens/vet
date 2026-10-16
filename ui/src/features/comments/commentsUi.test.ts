// The Comments UI's own transitions and subscriptions (`commentsUiModel.ts`), which the reactive
// store `commentsUi.svelte.ts` wraps; `pnpm test` runs the plain module without a component.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  type CommentsUpstream,
  clearReviewForm,
  closeOutgoing,
  closeReviewForm,
  followCommentsUi,
  initialCommentsUi,
  openOutgoing,
  showTab,
  toggleReviewForm,
} from "./commentsUiModel.ts";

function setup() {
  const listeners = new Set<() => void>();
  const upstream: CommentsUpstream = {
    onClose: (listener) => (listeners.add(listener), () => listeners.delete(listener)),
  };
  const state = initialCommentsUi();
  followCommentsUi(upstream, state);
  return {
    state,
    close: () => listeners.forEach((listener) => listener()),
  };
}

void test("opening the outgoing popover to post leaves the vote picker off", () => {
  const { state } = setup();
  openOutgoing(state, false);
  assert.equal(state.outgoingOpen, true);
  assert.equal(state.votePicking, false);
});

void test("opening the vote picker, and closing the popover, reset each other", () => {
  const { state } = setup();
  openOutgoing(state, true);
  assert.equal(state.outgoingOpen, true);
  assert.equal(state.votePicking, true);
  closeOutgoing(state);
  assert.equal(state.outgoingOpen, false);
  assert.equal(state.votePicking, false);
});

void test("the Comments tool window shows the tab it is asked for", () => {
  const { state } = setup();
  assert.equal(state.commentsTab, "review");
  showTab(state, "conversation");
  assert.equal(state.commentsTab, "conversation");
});

void test("a closed project has no outgoing popover up", () => {
  const { state, close } = setup();
  openOutgoing(state, true);
  close();
  assert.equal(state.outgoingOpen, false);
  assert.equal(state.votePicking, false);
});

void test("the review form opens and closes, keeping its words until it is submitted", () => {
  const { state } = setup();
  toggleReviewForm(state);
  assert.equal(state.review.open, true);
  state.review.body = "a summary";
  toggleReviewForm(state);
  assert.equal(state.review.open, false);
  toggleReviewForm(state);
  closeReviewForm(state);
  assert.equal(state.review.open, false);
  assert.equal(state.review.body, "a summary");
  clearReviewForm(state);
  assert.deepEqual(state.review, { open: false, verdict: undefined, body: "" });
});
