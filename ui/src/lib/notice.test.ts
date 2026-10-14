import assert from "node:assert/strict";
import { test } from "node:test";
import { abortError } from "./abort.ts";
import { STALE_NOTICE, failureNotice } from "./notice.ts";
import { RequestError } from "./requestError.ts";

void test("a request the changeset moved on from is the same notice whatever it was doing, whatever the backend said", () => {
  const error = new RequestError("outdated", "stale revision; the changeset is at revision 4");
  assert.equal(failureNotice("Could not load comments", error), STALE_NOTICE);
  assert.equal(failureNotice(undefined, error), STALE_NOTICE);
});

void test("another failure is told with what was being done, and a message that merely reads like staleness is no staleness", () => {
  assert.equal(
    failureNotice("Could not load comments", new RequestError("notFound", "no such thread 3")),
    "Could not load comments: no such thread 3",
  );
  assert.equal(failureNotice(undefined, new RequestError("internal", "stale revision")), "stale revision");
  assert.equal(failureNotice("Could not sync", "plain text"), "Could not sync: plain text");
});

void test("a request given up on is no failure to tell", () => {
  assert.equal(failureNotice("Could not load comments", abortError()), undefined);
});
