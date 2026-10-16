// The Pipelines tool's state for one session: which job is chosen and which runs are folded, the steps
// of the chosen job, and the actions on runs. Reading the runs is the checks store's; the platform does
// the work of an action, so this only asks and asks the checks again. The choice and the folds live as
// long as the session; the steps are the review screen's and go when it does.

import { SvelteSet } from "svelte/reactivity";
import type { PipelineAction } from "../generated/PipelineAction";
import type { PipelineStep } from "../generated/PipelineStep";
import { errorText } from "../lib/errorText";
import { toggleIn } from "../lib/foldKeys";
import type { Backend } from "./backend";
import { notices } from "./notices.svelte";
import { Latest } from "./resource";

/** What `act` says to the viewer when the platform took a cancel. */
const CANCELLING = "Cancelling. The run stops once the platform has stopped its jobs.";

class Pipelines {
  /** The job chosen in the tool, by check id. */
  selected = $state<string>();
  /** Runs shown without their jobs, by run id. */
  folded = new SvelteSet<string>();
  /** The steps of `job`, once the platform has answered. */
  steps = $state.raw<{ job: string; steps: PipelineStep[] }>();
  /** Why the steps cannot be shown, for the chosen job. */
  stepsError = $state<string>();
  /** A request to change a run is underway; no other goes out meanwhile. */
  busy = $state(false);
  /** The accounts whose token the platform said may not change runs, until the credentials change. */
  private denied = new SvelteSet<string>();
  private reads = new Latest();
  private shownFor?: { job: string; stamp: number | undefined };
  private backend: () => Backend | undefined;

  constructor(backend: () => Backend | undefined) {
    this.backend = backend;
  }

  /** Whether the platform refused the token of `account` a change of runs. */
  isDenied(account: string | undefined): boolean {
    return account !== undefined && this.denied.has(account);
  }

  /** The credentials changed: a refused token may be a different one now. */
  accountsChanged() {
    this.denied.clear();
  }

  toggle(run: string) {
    toggleIn(this.folded, run);
  }

  /** The review screen went away: the steps are dropped and the tool reads them again. */
  hideSteps() {
    this.reads.cancel();
    this.steps = undefined;
    this.stepsError = undefined;
    this.shownFor = undefined;
  }

  /**
   * Loads the steps of `job` (none clears them). A job that is over is read once; one that runs is
   * read again whenever the checks were (`stamp`), since its steps move on.
   */
  async showSteps(job: string | undefined, over: boolean, stamp: number | undefined) {
    const backend = this.backend();
    if (job === undefined || !backend) {
      this.hideSteps();
      return;
    }
    const known = this.shownFor;
    if (known?.job === job && (over || known.stamp === stamp) && this.stepsError === undefined) return;
    const isLatest = this.reads.begin();
    this.shownFor = { job, stamp };
    if (this.steps?.job !== job) this.steps = undefined;
    try {
      const steps = await backend.pipelineSteps(job);
      if (!isLatest()) return;
      this.steps = { job, steps };
      this.stepsError = undefined;
    } catch (error) {
      if (!isLatest()) return;
      this.steps = undefined;
      this.stepsError = errorText(error);
    }
  }

  /**
   * Asks the platform to change a run of `commit`'s checks; `account` is whose token it is. What the
   * platform refuses is told to the viewer, and a token it says may not do it turns the actions off.
   * The backend looks at the checks again after it took the request and pushes them.
   */
  async act(action: PipelineAction, account: string | undefined) {
    const backend = this.backend();
    if (this.busy || !backend) return;
    this.busy = true;
    try {
      const outcome = await backend.changePipeline(action);
      if (outcome.type === "accepted") {
        if (action.type === "cancel") notices.show(CANCELLING);
      } else if (outcome.type === "forbidden") {
        if (account !== undefined) this.denied.add(account);
        notices.show(`${outcome.message}. The actions stay off until the token changes.`);
      } else {
        notices.show(`Paused. ${outcome.message}`);
      }
    } catch (error) {
      notices.failed("Could not change the run", error);
    } finally {
      this.busy = false;
    }
  }
}

export type SessionPipelines = Pipelines;

export const newPipelines = (backend: () => Backend | undefined): SessionPipelines => new Pipelines(backend);
