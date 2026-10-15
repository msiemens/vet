// How the state of a commit's checks, or of one run, job or step, shows: an icon, a tone and a title.

import type { CheckPhase } from "../generated/CheckPhase";
import type { CommitChecks } from "../generated/CommitChecks";
import type { CiState } from "../generated/CiState";
import type { IconName } from "./ui/icons.ts";

export interface CiLook {
  icon: IconName;
  tone: "ok" | "failing" | "running" | "waiting";
  title: string;
}

/** A commit's checks in words, besides how they show. */
export interface CiStateLook extends CiLook {
  /** Where things stand, as the queue's header says it; none for checks that wait for approval. */
  phrase?: string;
  /** What the checks are doing, after a count: "3 failing". */
  word: string;
}

const CI_STATES = {
  success: { icon: "check", tone: "ok", title: "Checks passed", phrase: "Checks pass", word: "passed" },
  failure: { icon: "close", tone: "failing", title: "Checks failing", phrase: "Checks failing", word: "failing" },
  pending: { icon: "timeline", tone: "running", title: "Checks running", phrase: "Checks running", word: "running" },
  needsApproval: { icon: "lock", tone: "waiting", title: "Waiting for approval to run", word: "waiting" },
} as const satisfies Partial<Record<CiState, CiStateLook>>;

/** How passed checks show; what checks that decided nothing read as in words. */
export const CI_PASSED: CiStateLook = CI_STATES.success;

/** How the checks of a commit show; none for no checks, and for checks that decided nothing (neutral, skipped). */
export function ciIcon(ci: CiState | undefined): CiStateLook | undefined {
  return ci !== undefined && ci in CI_STATES ? CI_STATES[ci as keyof typeof CI_STATES] : undefined;
}

/** The tooltip of the header's checks button: what the checks say, and that they are old when the platform limits requests. */
export function headerTitle(checks: CommitChecks, limited?: { until: string; asOf: string; platform: string }): string {
  const failed = checks.runs.filter((r) => r.state === "failure").map((r) => r.name);
  const { title } = ciIcon(checks.state) ?? CI_PASSED;
  const base = checks.state === "failure" ? `${title}: ${failed.join(", ")}` : title;
  return limited
    ? `${base}. ${limited.platform} is limiting requests until ${limited.until}; as of ${limited.asOf}.`
    : base;
}

/** How a run, job or step shows: its phase while it waits, runs or was stopped, else how it ended. */
export function phaseLook(state: CiState, phase: CheckPhase): CiLook {
  switch (phase) {
    case "queued":
      return { icon: "timeline", tone: "waiting", title: "Queued" };
    case "running":
      return { icon: "timeline", tone: "running", title: "Running" };
    case "cancelled":
      return { icon: "minus", tone: "waiting", title: "Cancelled" };
    case "finished":
      break;
  }
  switch (state) {
    case "success":
      return { icon: "check", tone: "ok", title: "Passed" };
    case "failure":
      return { icon: "close", tone: "failing", title: "Failed" };
    case "needsApproval":
      return { icon: "lock", tone: "running", title: "Needs approval" };
    default:
      return { icon: "minus", tone: "waiting", title: "Skipped" };
  }
}
