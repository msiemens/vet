import { InFlight } from "./inFlight.ts";

/** What opening a repository does to the window and the backend; the session store provides it. */
export interface OpenSteps {
  /** Lets go of the open repository, if any. */
  release(): Promise<void>;
  /** Opens a project; resolves with the repository root the backend settled on. */
  open(path: string): Promise<string>;
  setWindowMode(mode: "welcome" | "project"): Promise<void>;
  opened(root: string): void;
  failed(path: string, error: unknown): void;
}

/**
 * Opens repositories one request at a time per path: asking for a path that is being opened joins
 * that open, so a double click or a held Enter sends one request to the backend.
 */
export class RepoOpener {
  private opening = new InFlight<void>();
  private steps: OpenSteps;

  constructor(steps: OpenSteps) {
    this.steps = steps;
  }

  open(path: string): Promise<void> {
    return this.opening.share(path, async () => {
      const { steps } = this;
      await steps.release();
      try {
        const root = await steps.open(path);
        await steps.setWindowMode("project");
        steps.opened(root);
      } catch (error) {
        steps.failed(path, error);
        await steps.setWindowMode("welcome").catch(console.error);
      }
    });
  }
}
