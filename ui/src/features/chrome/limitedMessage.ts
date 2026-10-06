import type { RestoreFailure } from "../../generated/RestoreFailure";

/** What the viewer is told when a dependency restore failed: what needs those packages cannot be resolved. */
export function limitedMessage(failures: RestoreFailure[]): string {
  const projects = new Set(failures.map((f) => f.project)).size;
  const where = projects === 1 ? `project ${failures[0].project}` : `${projects} projects`;
  return `Analysis limited: the dependencies of ${where} could not be installed, so symbols from them stay unresolved.`;
}
