// When the Pipelines tool reads the steps of the chosen job again.
import type { CheckRun } from "../generated/CheckRun";

/** What the steps on show were read for: the job, and the record of it the checks held then. */
export interface StepsRead {
  job: string;
  run: CheckRun;
}

/**
 * Whether the steps of `run` are to be read: nothing was read for its job yet, the last read failed, or the
 * job still runs and the checks replaced its record (every push of the checks brings new records).
 */
export function readSteps(held: StepsRead | undefined, run: CheckRun, over: boolean, failed: boolean): boolean {
  if (held?.job !== run.id || failed) return true;
  return !over && held.run !== run;
}
