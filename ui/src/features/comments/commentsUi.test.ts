// The Comments UI's own transitions and subscriptions (`commentsUiModel.ts`), which the reactive
// store `commentsUi.svelte.ts` wraps; `pnpm test` runs the plain module without a component.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Opening } from "../../state/changeset.svelte.ts";
import {
  type CommentsUpstream,
  cancelChangesetComment,
  clearChangesetComment,
  clearReviewForm,
  closeOutgoing,
  closeReviewForm,
  composeChangesetComment,
  followCommentsUi,
  initialCommentsUi,
  openOutgoing,
  showTab,
  toggleReviewForm,
} from "./commentsUiModel.ts";

function setup() {
  const listeners = {
    opening: new Set<(opening: Opening) => void>(),
    close: new Set<() => void>(),
  };
  const upstream: CommentsUpstream = {
    onOpening: (listener) => (listeners.opening.add(listener), () => listeners.opening.delete(listener)),
    onClose: (listener) => (listeners.close.add(listener), () => listeners.close.delete(listener)),
  };
  const state = initialCommentsUi();
  followCommentsUi(upstream, state);
  return {
    state,
    opening: (opening: Opening) => listeners.opening.forEach((listener) => listener(opening)),
    close: () => listeners.close.forEach((listener) => listener()),
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

void test("the changeset comment form cancels keeping the words, and clears when posted", () => {
  const { state } = setup();
  assert.equal(state.changesetComment.composing, false);
  composeChangesetComment(state);
  state.changesetComment.body = "a word";
  assert.equal(state.changesetComment.composing, true);
  cancelChangesetComment(state);
  assert.deepEqual(state.changesetComment, { composing: false, body: "a word" });
  clearChangesetComment(state);
  assert.deepEqual(state.changesetComment, { composing: false, body: "" });
});

void test("another pull request starts with no changeset comment being written", () => {
  const { state, opening } = setup();
  composeChangesetComment(state);
  state.changesetComment.body = "a word";
  // An ask alone keeps the form: only the announced changeset is another pull request.
  opening({ type: "requested" });
  assert.equal(state.changesetComment.composing, true);
  opening({ type: "opened", changeset: { type: "workingTree" } });
  assert.deepEqual(state.changesetComment, { composing: false, body: "" });
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
