import type { RestoreFailure } from "../../generated/RestoreFailure";

/** Whether every entry is a stand-in that installed the dependencies, none a failure. */
export function onlyStoodIn(failures: RestoreFailure[]): boolean {
  return failures.every((f) => f.kind === "stoodIn");
}

/** What the viewer is told about the restores: that one failed, so what needs those packages cannot be resolved, or only that a stand-in did the install. */
export function limitedMessage(failures: RestoreFailure[]): string {
  if (onlyStoodIn(failures)) return "Dependencies installed with npm install: the lock file is out of sync.";
  const failed = failures.filter((f) => f.kind === "failed");
  const projects = new Set(failed.map((f) => f.project)).size;
  const where = projects === 1 ? `project ${failed[0].project}` : `${projects} projects`;
  return `Analysis limited: the dependencies of ${where} could not be installed, so symbols from them stay unresolved.`;
}
