// What the comment boxes hold that was not added as a comment, and how many of them would lose it.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CommentDrafts,
  cancelChangesetComment,
  clearChangesetComment,
  composeChangesetComment,
  emptyChangesetComment,
  unsentComments,
} from "./commentDrafts.ts";

void test("the changeset comment form cancels keeping the words, and clears when posted", () => {
  const comment = emptyChangesetComment();
  composeChangesetComment(comment);
  comment.body = "a word";
  assert.equal(comment.composing, true);
  cancelChangesetComment(comment);
  assert.deepEqual(comment, { composing: false, body: "a word" });
  clearChangesetComment(comment);
  assert.deepEqual(comment, emptyChangesetComment());
});

void test("only boxes with text count, and an edit form only once its text differs from the message's", () => {
  const drafts = new CommentDrafts();
  const none = emptyChangesetComment();
  const held = () => unsentComments(drafts.count(), none);
  assert.equal(held(), 0);
  drafts.set("reply:3", "   ");
  drafts.begin("edit:7", "the message");
  assert.equal(held(), 0, "a blank reply and an untouched edit hold nothing");
  drafts.set("reply:4", "a reply");
  drafts.set("edit:7", "the message, edited");
  assert.equal(held(), 2);
  drafts.delete("edit:7");
  drafts.delete("reply:4");
  assert.equal(held(), 0);
});

void test("the changeset comment counts while it holds words, also with its form closed", () => {
  const drafts = new CommentDrafts();
  const comment = emptyChangesetComment();
  composeChangesetComment(comment);
  assert.equal(unsentComments(drafts.count(), comment), 0);
  comment.body = "a word";
  cancelChangesetComment(comment);
  drafts.set("new:x", "on code");
  assert.equal(unsentComments(drafts.count(), comment), 2);
});
