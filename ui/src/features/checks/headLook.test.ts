import assert from "node:assert/strict";
import { test } from "node:test";
import type { CheckRun } from "../../generated/CheckRun";
import type { CommitChecks } from "../../generated/CommitChecks";
import type { PlatformStatus } from "../../generated/PlatformStatus";
import { headChecksLook, pipelineBadge } from "./headLook.ts";

const run = (state: CheckRun["state"], name = "job"): CheckRun => ({ state, name }) as CheckRun;

const checks = (state: CommitChecks["state"], runs: CheckRun[] = []): CommitChecks => ({
  commit: "a".repeat(40),
  state,
  runs,
  pipelines: [],
});

const status = (kind: PlatformStatus["kind"]): PlatformStatus => ({
  host: "github.com",
  kind,
  connection: { type: "ok" },
});

void test("a commit without checks has no look", () => {
  assert.equal(headChecksLook(undefined, undefined, false, undefined, undefined), undefined);
});

void test("the title says what the checks say, and without a limit nothing is stale", () => {
  const head = checks("failure", [run("failure", "a"), run("failure", "b")]);
  const look = headChecksLook(head, undefined, false, undefined, undefined);
  assert.ok(look);
  assert.equal(look.head, head);
  assert.equal(look.title, "Checks failing: a, b");
  assert.equal(look.stale, false);
});

void test("signed out, the title says the token is not valid", () => {
  const look = headChecksLook(checks("success"), status("github"), true, undefined, undefined);
  assert.ok(look);
  assert.equal(look.stale, false);
  assert.match(look.title, /GitHub: the token for github\.com is not valid/);
});

void test("a platform that limits requests makes the look stale and names the platform", () => {
  const look = headChecksLook(checks("pending"), status("github"), false, 1_000_000, 999_000);
  assert.ok(look);
  assert.equal(look.stale, true);
  assert.match(look.title, /GitHub is limiting requests until/);
});

void test("the Pipelines badge counts what the head's checks still need, and is quiet when all passed", () => {
  assert.deepEqual(pipelineBadge(undefined, false), { count: 0, text: "" });
  assert.deepEqual(pipelineBadge(checks("failure", [run("failure"), run("failure"), run("success")]), false), {
    count: 2,
    text: "2 failing",
    bad: true,
  });
  assert.deepEqual(pipelineBadge(checks("pending", [run("pending"), run("success")]), false), {
    count: 1,
    text: "1 running",
    bad: false,
  });
  assert.deepEqual(pipelineBadge(checks("success", [run("success")]), false), { count: 0, text: "" });
  assert.deepEqual(pipelineBadge(checks(undefined, []), false), { count: 0, text: "" });
  assert.deepEqual(pipelineBadge(checks("failure", [run("failure")]), true), { count: 0, text: "" }, "signed out");
});
