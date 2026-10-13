import assert from "node:assert/strict";
import { test } from "node:test";
import type { CheckRun } from "../../generated/CheckRun.ts";
import type { CommitChecks } from "../../generated/CommitChecks.ts";
import type { Pipeline } from "../../generated/Pipeline.ts";
import {
  defaultJob,
  disabledReason,
  headerTitle,
  logKey,
  look,
  rerunnable,
  runMeta,
  runOf,
  runRows,
} from "./pipelineRows.ts";

const pipeline = (id: string, phase: Pipeline["phase"], state: Pipeline["state"], attempt = 1): Pipeline => ({
  id,
  name: `workflow ${id}`,
  detail: "#42 · pull_request",
  attempt,
  state,
  phase,
});
const job = (id: string, over: Partial<CheckRun> = {}): CheckRun => ({
  id,
  name: `job ${id}`,
  state: "success",
  phase: "finished",
  pipeline: "a",
  log: true,
  ...over,
});
const commit = (runs: CheckRun[], pipelines: Pipeline[]): CommitChecks => ({ commit: "abc", runs, pipelines });

void test("jobs are listed under their run and what a service set stands apart", () => {
  const checks = commit(
    [job("1", { pipeline: "b" }), job("2", { pipeline: undefined }), job("3", { pipeline: "a" })],
    [pipeline("a", "finished", "success"), pipeline("b", "finished", "success")],
  );
  const { runs, services } = runRows(checks);
  assert.deepEqual(
    runs.map((r) => [r.run.id, r.jobs.map((j) => j.id)]),
    [
      ["a", ["3"]],
      ["b", ["1"]],
    ],
  );
  assert.deepEqual(
    services.map((s) => s.id),
    ["2"],
  );
  assert.equal(runOf(checks, "1")?.id, "b");
  assert.equal(runOf(checks, "2"), undefined);
});

void test("the tool opens on the job that failed, else the one that runs", () => {
  const ps = [pipeline("a", "running", "pending")];
  const failed = commit([job("1"), job("2", { state: "failure" }), job("3", { phase: "running" })], ps);
  assert.equal(defaultJob(failed), "2");
  const running = commit([job("1"), job("3", { phase: "running", state: "pending" })], ps);
  assert.equal(defaultJob(running), "3");
  assert.equal(defaultJob(commit([job("9", { pipeline: undefined })], [])), undefined);
});

void test("a run is re-run when it is over and has something that failed, and stopped while it is not over", () => {
  const failing = [job("1", { state: "failure" }), job("2")];
  const over = pipeline("a", "finished", "failure");
  const going = pipeline("a", "running", "pending");
  const on = (run: Pipeline | undefined, jobs: CheckRun[], extra = {}) => ({ run, jobs, busy: false, ...extra });

  assert.equal(disabledReason("rerunFailed", on(over, failing)), undefined);
  assert.equal(disabledReason("rerunFailed", on(over, [job("2")])), "Nothing failed in this run.");
  assert.equal(disabledReason("rerunFailed", on(going, failing)), "Wait until the run finishes.");
  assert.equal(disabledReason("rerunAll", on(over, failing)), undefined);
  assert.equal(disabledReason("rerunAll", on(going, failing)), "Wait until the run finishes.");
  assert.equal(disabledReason("cancel", on(going, failing)), undefined);
  assert.equal(disabledReason("cancel", on(over, failing)), "Nothing is running.");
  assert.equal(disabledReason("cancel", on(undefined, [])), "Nothing loaded");
});

void test("a limit, a refused token or a request underway holds every action back with its reason", () => {
  const run = pipeline("a", "finished", "failure");
  const jobs = [job("1", { state: "failure" })];
  const held = disabledReason("rerunAll", { run, jobs, held: "GitHub is limiting requests until 14:32.", busy: true });
  assert.equal(held, "GitHub is limiting requests until 14:32.");
  assert.equal(disabledReason("rerunAll", { run, jobs, busy: true }), "Waiting for the platform");
});

void test("a job is re-run on its own when it failed or was stopped and its run is over", () => {
  const over = pipeline("a", "finished", "failure");
  assert.equal(rerunnable(job("1", { state: "failure" }), over), true);
  assert.equal(rerunnable(job("1", { phase: "cancelled", state: "neutral" }), over), true);
  assert.equal(rerunnable(job("1"), over), false);
  assert.equal(rerunnable(job("1", { state: "failure" }), pipeline("a", "running", "pending")), false);
  assert.equal(rerunnable(job("1", { state: "failure" }), undefined), false);
});

void test("a job reads by how far it has come before how it ended", () => {
  assert.equal(look("pending", "queued"), "queued");
  assert.equal(look("pending", "running"), "running");
  assert.equal(look("neutral", "cancelled"), "cancelled");
  assert.equal(look("success", "finished"), "success");
  assert.equal(look("failure", "finished"), "failure");
  assert.equal(look("needsApproval", "finished"), "approval");
  assert.equal(look("skipped", "finished"), "skipped");
});

void test("the button's tooltip names what failed, and says the result is old while the platform limits requests", () => {
  const checks = commit([job("1", { state: "failure" }), job("2", { state: "failure" })], []);
  const failing = { ...checks, state: "failure" as const };
  assert.equal(headerTitle(failing), "Checks failing: job 1, job 2");
  assert.equal(
    headerTitle({ ...failing, state: "pending" }, { until: "14:32", asOf: "14:21", platform: "GitHub" }),
    "Checks running. GitHub is limiting requests until 14:32; as of 14:21.",
  );
});

void test("a re-run run has a log of its own, named by its attempt", () => {
  assert.deepEqual(logKey(job("7"), pipeline("a", "finished", "success")), { key: "7", title: "job 7 · log" });
  assert.deepEqual(logKey(job("7"), pipeline("a", "finished", "success", 2)), {
    key: "7@2",
    title: "job 7 · log (attempt 2)",
  });
  assert.equal(runMeta(pipeline("a", "finished", "success", 2)), "#42 · pull_request · attempt 2");
});
