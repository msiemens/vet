// How the state of a commit's checks shows: an icon, a tone and a title.

import type { CiState } from "../generated/CiState";
import type { IconName } from "./ui/icons.ts";

export interface CiLook {
  icon: IconName;
  tone: "ok" | "failing" | "running" | "attention";
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
      return { icon: "lock", tone: "attention", title: "Waiting for approval to run" };
    default:
      return undefined;
  }
}
