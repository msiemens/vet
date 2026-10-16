// The Pipelines tool's state: which job is chosen and which runs are folded, the steps of the chosen
// job, and the actions on runs. Reading the runs is the checks store's; the platform does the work
// of an action, so this only asks and asks the checks again.

import { SvelteSet } from "svelte/reactivity";
import type { PipelineAction } from "../generated/PipelineAction";
import type { PipelineStep } from "../generated/PipelineStep";
import { errorText } from "../lib/errorText";
import { toggleIn } from "../lib/foldKeys";
import { whileReview } from "./reviewFollow";
import { notices } from "./notices.svelte";
import { Latest } from "./resource";
import { session } from "./session.svelte";

/** What `act` says to the viewer when the platform took a cancel. */
const CANCELLING = "Cancelling. The run stops once the platform has stopped its jobs.";

class PipelineStore {
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

  constructor() {
    session.onConnect((backend) =>
      backend.onAccountsChanged(() => {
        this.denied.clear();
      }),
    );
    // The steps are the review screen's; leaving it drops them and the tool reads them again.
    whileReview(() => () => {
      this.reads.cancel();
      this.steps = undefined;
      this.stepsError = undefined;
      this.shownFor = undefined;
    });
    session.onClose(() => {
      this.selected = undefined;
      this.steps = undefined;
      this.shownFor = undefined;
      this.folded.clear();
      this.denied.clear();
    });
  }

  /** Whether the platform refused the token of `account` a change of runs. */
  isDenied(account: string | undefined): boolean {
    return account !== undefined && this.denied.has(account);
  }

  toggle(run: string) {
    toggleIn(this.folded, run);
  }

  /**
   * Loads the steps of `job` (none clears them). A job that is over is read once; one that runs is
   * read again whenever the checks were (`stamp`), since its steps move on.
   */
  async showSteps(job: string | undefined, over: boolean, stamp: number | undefined) {
    const backend = session.backend;
    if (job === undefined || !backend) {
      this.reads.cancel();
      this.steps = undefined;
      this.stepsError = undefined;
      this.shownFor = undefined;
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
    const backend = session.backend;
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

export const pipelines = new PipelineStore();
