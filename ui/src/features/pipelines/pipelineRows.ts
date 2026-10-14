// What the Pipelines tool lists and which of its actions are on, from the checks of a commit.

import type { CheckPhase } from "../../generated/CheckPhase";
import type { CheckRun } from "../../generated/CheckRun";
import type { CommitChecks } from "../../generated/CommitChecks";
import type { Pipeline } from "../../generated/Pipeline";
import { CI_PASSED, ciIcon } from "../../lib/ciStatus.ts";

/** A run with its jobs, in the order the platform listed them. */
export interface RunRows {
  run: Pipeline;
  jobs: CheckRun[];
}

/** The commit's checks as the tool lists them: its runs with their jobs, then what services set. */
export function runRows(checks: CommitChecks): { runs: RunRows[]; services: CheckRun[] } {
  const runs: RunRows[] = checks.pipelines.map((run) => ({
    run,
    jobs: checks.runs.filter((job) => job.pipeline === run.id),
  }));
  return { runs, services: checks.runs.filter((job) => job.pipeline === undefined) };
}

/** The run a job belongs to. */
export function runOf(checks: CommitChecks, job: string | undefined): Pipeline | undefined {
  const pipeline = checks.runs.find((r) => r.id === job)?.pipeline;
  return checks.pipelines.find((p) => p.id === pipeline);
}

/** The job the tool opens on: the first that failed, else the first that runs, else the first. */
export function defaultJob(checks: CommitChecks): string | undefined {
  const jobs = checks.runs.filter((r) => r.pipeline !== undefined);
  return (jobs.find((j) => j.state === "failure") ?? jobs.find((j) => j.phase === "running") ?? jobs.at(0))?.id;
}

/** Whether the tool stays open for a changeset of this kind; one that is still loading (none yet) keeps it. */
export const belongsTo = (changesetType: string | undefined): boolean =>
  changesetType === undefined || changesetType === "pullRequest";

/** A run or job that is not going to change by itself. */
export const finished = (phase: CheckPhase) => phase === "finished" || phase === "cancelled";

export type ActionKind = "rerunFailed" | "rerunAll" | "cancel";

/** What stands between the viewer and an action. */
export interface ActionContext {
  /** The run the action is on; none when nothing is loaded. */
  run?: Pipeline;
  jobs: readonly CheckRun[];
  /** Why every action waits: the platform limits requests, or the token cannot change runs. */
  held?: string;
  /** A request of the viewer is underway. */
  busy: boolean;
}

/** Why the action is off, if it is. */
export function disabledReason(kind: ActionKind, { run, jobs, held, busy }: ActionContext): string | undefined {
  if (run === undefined) return "Nothing loaded";
  if (held !== undefined) return held;
  if (busy) return "Waiting for the platform";
  const done = finished(run.phase);
  switch (kind) {
    case "rerunFailed":
      if (!jobs.some((j) => j.state === "failure")) return "Nothing failed in this run.";
      return done ? undefined : "Wait until the run finishes.";
    case "rerunAll":
      return done ? undefined : "Wait until the run finishes.";
    case "cancel":
      return done ? "Nothing is running." : undefined;
  }
}

/** Whether a job can be run again on its own: it failed or was stopped, and its run is over. */
export function rerunnable(job: CheckRun, run: Pipeline | undefined): boolean {
  return (job.state === "failure" || job.phase === "cancelled") && run !== undefined && finished(run.phase);
}

/** The tooltip of the header's button: what the checks say, and that they are old when the platform limits requests. */
export function headerTitle(checks: CommitChecks, limited?: { until: string; asOf: string; platform: string }): string {
  const failed = checks.runs.filter((r) => r.state === "failure").map((r) => r.name);
  const { title } = ciIcon(checks.state) ?? CI_PASSED;
  const base = checks.state === "failure" ? `${title}: ${failed.join(", ")}` : title;
  return limited
    ? `${base}. ${limited.platform} is limiting requests until ${limited.until}; as of ${limited.asOf}.`
    : base;
}

/** A run's line beside its name: the platform's number and why it ran, and the attempt after a re-run. */
export function runMeta(run: Pipeline): string {
  return run.attempt > 1 ? `${run.detail} · attempt ${run.attempt}` : run.detail;
}

/** A log tab's id part and title for a job: a re-run job has the log of another attempt. */
export function logKey(job: CheckRun, run: Pipeline | undefined): { key: string; title: string } {
  const attempt = run?.attempt ?? 1;
  return attempt > 1
    ? { key: `${job.id}@${attempt}`, title: `${job.name} · log (attempt ${attempt})` }
    : { key: job.id, title: `${job.name} · log` };
}
