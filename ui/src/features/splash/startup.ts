// What the splash shows for the host's startup progress. The host reports the step it is in;
// the splash names it and fills the bar with the steps behind it, never with a guessed share
// of the current one.

import type { StartupProgress, StartupStep } from "../../state/host.ts";

const STEPS: { step: StartupStep; label: string; showsTarget: boolean }[] = [
  { step: "startingBackend", label: "Starting backend", showsTarget: true },
  { step: "waitingForReady", label: "Waiting for backend", showsTarget: true },
  { step: "loadingUi", label: "Loading interface", showsTarget: false },
];

export interface StartupView {
  label: string;
  /** Where the backend runs, shown beside the label. */
  target: string | null;
  /** Steps behind the current one, out of `total`. */
  done: number;
  total: number;
  error: string | null;
}

export function describe(progress: StartupProgress): StartupView {
  const done = STEPS.findIndex((s) => s.step === progress.step);
  const { label, showsTarget } = STEPS[done];
  return {
    label,
    target: showsTarget ? (progress.distro === null ? "local" : `WSL · ${progress.distro}`) : null,
    done,
    total: STEPS.length,
    error: progress.error,
  };
}
