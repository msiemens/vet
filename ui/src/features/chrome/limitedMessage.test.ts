import assert from "node:assert/strict";
import { test } from "node:test";
import { limitedMessage } from "./limitedMessage.ts";

const failure = (project: string, tool = "pnpm") => ({ project, tool, message: "failed" });

void test("names the project when one failed, counts them when several did", () => {
  assert.match(limitedMessage([failure("web")]), /project web /);
  assert.match(limitedMessage([failure("web"), failure("web", "yarn")]), /project web /);
  assert.match(limitedMessage([failure("web"), failure("api")]), /2 projects/);
});
