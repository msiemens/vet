import assert from "node:assert/strict";
import { test } from "node:test";
import type { Notification } from "../../generated/Notification.ts";
import { notificationCopyText, notificationText } from "./notificationText.ts";

void test("a deleted thread names its place and platform, and says when a draft went with it", () => {
  const gone: Notification = { type: "threadDeleted", place: "a.txt:21", platform: "github", dropped: [] };
  assert.equal(notificationText(gone), "Thread on a.txt:21 was deleted on GitHub.");
  assert.equal(notificationCopyText(gone), null, "nothing was dropped to copy");

  const withDraft: Notification = {
    type: "threadDeleted",
    place: "b.txt:4",
    platform: "azureDevOps",
    dropped: ["Then use 48 h.", "Check 21 too."],
  };
  assert.equal(
    notificationText(withDraft),
    "Thread on b.txt:4 was deleted on Azure DevOps. Your draft reply was removed.",
  );
  assert.equal(notificationCopyText(withDraft), "Then use 48 h.\n\nCheck 21 too.");
});

void test("a comment that was not posted says how many texts wait", () => {
  const one: Notification = {
    type: "commentsNotPosted",
    notPosted: ["Fix the header."],
    problem: "the platform refused it",
  };
  assert.equal(notificationText(one), "A comment was not posted: the platform refused it");
  assert.equal(notificationCopyText(one), "Fix the header.");

  const several: Notification = { type: "commentsNotPosted", notPosted: ["One.", "Two."], problem: "offline" };
  assert.equal(notificationText(several), "2 comments were not posted: offline");
  assert.equal(notificationCopyText(several), "One.\n\nTwo.");

  const whole: Notification = { type: "commentsNotPosted", notPosted: [], problem: "the platform refused it" };
  assert.equal(notificationText(whole), "A comment was not posted: the platform refused it");
  assert.equal(notificationCopyText(whole), null, "nothing was left to copy");
});

void test("a vote that was not cast says why", () => {
  const notice: Notification = { type: "voteNotCast", problem: "the platform could not be reached" };
  assert.equal(notificationText(notice), "The vote was not cast: the platform could not be reached");
  assert.equal(notificationCopyText(notice), null);
});

void test("a language server that died says whether it was restarted or stays stopped", () => {
  const restarted: Notification = { type: "serverDied", name: "rust-analyzer", outcome: "restarted" };
  assert.equal(notificationText(restarted), "rust-analyzer stopped unexpectedly and was restarted.");
  assert.equal(notificationCopyText(restarted), null);

  const stopped: Notification = { type: "serverDied", name: "rust-analyzer", outcome: "stopped" };
  assert.equal(
    notificationText(stopped),
    "rust-analyzer stopped unexpectedly again and stays stopped. Restart it from the Help menu.",
  );
});

void test("a stopped sync names the host and why", () => {
  const signedOut: Notification = { type: "syncStopped", host: "github.com", reason: "signedOut" };
  assert.equal(notificationText(signedOut), "Not signed in to github.com: comments are not synced.");
  assert.equal(notificationCopyText(signedOut), null);

  const expired: Notification = { type: "syncStopped", host: "dev.azure.com", reason: "tokenExpired" };
  assert.equal(
    notificationText(expired),
    "The token for dev.azure.com is no longer accepted: comments are not synced.",
  );
});
