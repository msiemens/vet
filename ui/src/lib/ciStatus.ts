// How the state of a commit's checks, or of one run, job or step, shows: an icon, a tone and a title.

import type { CheckPhase } from "../generated/CheckPhase";
import type { CiState } from "../generated/CiState";
import type { IconName } from "./ui/icons.ts";

export interface CiLook {
  icon: IconName;
  tone: "ok" | "failing" | "running" | "waiting";
  title: string;
}

/** How the checks of a commit show; none for no checks, and for checks that decided nothing (neutral, skipped). */
export function ciIcon(ci: CiState | undefined): CiLook | undefined {
  switch (ci) {
    case "success":
      return { icon: "check", tone: "ok", title: "Checks passed" };
    case "failure":
      return { icon: "close", tone: "failing", title: "Checks failing" };
    case "pending":
      return { icon: "timeline", tone: "running", title: "Checks running" };
    case "needsApproval":
      return { icon: "lock", tone: "waiting", title: "Waiting for approval to run" };
    default:
      return undefined;
  }
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
