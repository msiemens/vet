// The ways to close a review as the UI gives them: one test each so a wording and the name
// it is found again under cannot drift apart unnoticed.

import assert from "node:assert/strict";
import { test } from "node:test";
import { verdictId, verdictLabel, verdictOf } from "./verdicts.ts";

void test("every verdict reads as it did, wherever a platform has it; an unknown one as what it was spelled", () => {
  for (const [verdict, label] of [
    [{ type: "comment" }, "Comment"],
    [{ type: "approve" }, "Approve"],
    [{ type: "requestChanges" }, "Request changes"],
    [{ type: "approveWithSuggestions" }, "Approve with suggestions"],
    [{ type: "waitForAuthor" }, "Wait for author"],
    [{ type: "reject" }, "Reject"],
    [{ type: "other", value: "waiting" }, "waiting"],
  ] as const) {
    assert.equal(verdictLabel(verdict), label, `${verdict.type} reads`);
  }
});

void test("a verdict's id finds the verdict again; a platform's e.g. an unknown spelling does not come back one", () => {
  for (const id of ["comment", "approve", "requestChanges", "approveWithSuggestions", "waitForAuthor", "reject"]) {
    const verdict = verdictOf(id);
    assert.ok(verdict, id);
    assert.equal(verdict.type, id);
    assert.equal(verdictId(verdict), id);
  }
  assert.equal(verdictOf("approved"), undefined, "no platform's spelling is an id now");
  assert.equal(verdictOf("strange new verdict"), undefined);
});
