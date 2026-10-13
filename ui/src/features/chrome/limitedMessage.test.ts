import assert from "node:assert/strict";
import { test } from "node:test";
import type { RestoreFailure } from "../../generated/RestoreFailure.ts";
import { limitedMessage } from "./limitedMessage.ts";

const failure = (project: string, tool = "pnpm"): RestoreFailure => ({
  kind: "failed",
  project,
  tool,
  message: "failed",
});
const standIn = (project: string): RestoreFailure => ({ kind: "stoodIn", project, tool: "npm", message: "note" });

void test("names the project when one failed, counts them when several did", () => {
  assert.match(limitedMessage([failure("web")]), /project web /);
  assert.match(limitedMessage([failure("web"), failure("web", "yarn")]), /project web /);
  assert.match(limitedMessage([failure("web"), failure("api")]), /2 projects/);
});

void test("only stand-ins get the milder headline", () => {
  assert.equal(
    limitedMessage([standIn("web"), standIn("api")]),
    "Dependencies installed with npm install: the lock file is out of sync.",
  );
});

void test("a failure among stand-ins keeps the failure headline, counting only the failed projects", () => {
  const message = limitedMessage([standIn("web"), failure("api")]);
  assert.match(message, /^Analysis limited: .*project api /);
});
