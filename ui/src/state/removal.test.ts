import assert from "node:assert/strict";
import { test } from "node:test";
import { lostItems, removeQuestion } from "./removal.ts";

void test("what is lost lists open reviews, then unposted comments, each with its count", () => {
  assert.deepEqual(lostItems({ openReviews: 2, unpostedComments: 3 }), ["2 open reviews", "3 unposted comments"]);
  assert.deepEqual(lostItems({ openReviews: 1, unpostedComments: 0 }), ["1 open review"]);
  assert.deepEqual(lostItems({ openReviews: 0, unpostedComments: 1 }), ["1 unposted comment"]);
});

void test("nothing is listed when nothing would be lost", () => {
  assert.deepEqual(lostItems({ openReviews: 0, unpostedComments: 0 }), []);
});

void test("the question names the project and leads into the list only when there is one", () => {
  assert.equal(removeQuestion("billing-api", []), "Do you want to remove project billing-api?");
  assert.equal(
    removeQuestion("billing-api", ["1 open review"]),
    "Do you want to remove project billing-api? The following will be lost:",
  );
});
