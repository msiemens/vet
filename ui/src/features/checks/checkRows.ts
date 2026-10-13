// What the checks of a commit say in words and numbers: the toolbar's badge, the commit header's
// "4/4" and the popover's durations.

import type { CheckRun } from "../../generated/CheckRun";
import type { CommitChecks } from "../../generated/CommitChecks";

/** A run that did not hold the commit back. */
const passes = (run: CheckRun) => run.state === "success" || run.state === "neutral" || run.state === "skipped";

/** How many runs passed, and how many there are. */
export function passed(checks: CommitChecks): { passed: number; total: number } {
  return { passed: checks.runs.filter(passes).length, total: checks.runs.length };
}

/**
 * The number beside the toolbar's icon: the runs that failed while some failed, else those still
 * running, else those waiting for approval, else those that passed.
 */
export function badgeCount(checks: CommitChecks): number {
  const count = (state: CheckRun["state"]) => checks.runs.filter((r) => r.state === state).length;
  if (checks.state === "failure") return count("failure");
  if (checks.state === "pending") return count("pending");
  if (checks.state === "needsApproval") return count("needsApproval");
  return passed(checks).passed;
}

/** A run's duration as `4m 40s`, `12s` or `1h 02m`. */
export function durationText(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  if (seconds < 60) return `${seconds}s`;
  if (minutes < 60) return `${minutes}m ${String(seconds % 60).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}
