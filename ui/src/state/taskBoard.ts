// The background tasks the backend announced and has not ended, and which of them are worth
// showing yet: a step that is over in a blink (a rescan after an edit) never reaches the footer.

import type { Task } from "../generated/Task";

interface Entry {
  task: Task;
  /** The time from which the task is shown. */
  shownFrom: number;
}

/** The kinds whose last failure stays shown until a task of the kind finishes. */
const LASTING: readonly Task["kind"][] = ["sync", "checks", "remoteFetch"];

/** The lasting failures that belong to the changeset on view. */
const PER_CHANGESET: readonly Task["kind"][] = ["sync", "checks"];

export class TaskBoard {
  private entries = new Map<number, Entry>();
  private failures = new Map<Task["kind"], Task>();

  /**
   * A task was announced at `now`. A new task is shown from `now + delay`; a running one keeps
   * the time it was first shown from. An ended task is dropped; a failed one of a lasting kind is
   * kept as a failure until a task of its kind finishes.
   */
  apply(task: Task, now: number, delay: number) {
    if (task.state.type !== "running") {
      this.entries.delete(task.id);
      if (LASTING.includes(task.kind)) {
        if (task.state.type === "failed") this.failures.set(task.kind, task);
        else this.failures.delete(task.kind);
      }
      return;
    }
    const shownFrom = this.entries.get(task.id)?.shownFrom ?? now + delay;
    this.entries.set(task.id, { task, shownFrom });
  }

  /** The running tasks as the backend lists them now, which replaces what was known. */
  replace(tasks: Task[], now: number, delay: number) {
    const before = this.entries;
    this.entries = new Map();
    for (const task of tasks) {
      if (task.state.type !== "running") continue;
      this.entries.set(task.id, { task, shownFrom: before.get(task.id)?.shownFrom ?? now + delay });
    }
  }

  clear() {
    this.entries.clear();
    this.failures.clear();
  }

  /** Drops the lasting failures of the changeset on view; the running tasks go on. */
  clearFailures() {
    for (const kind of PER_CHANGESET) this.failures.delete(kind);
  }

  /** The failures that stay shown, oldest first. */
  failed(): Task[] {
    return [...this.failures.values()].sort((a, b) => a.id - b.id);
  }

  /** The tasks to show at `now`, oldest first. */
  shown(now: number): Task[] {
    return [...this.entries.values()]
      .filter((e) => e.shownFrom <= now)
      .map((e) => e.task)
      .sort((a, b) => a.id - b.id);
  }

  /** How long until another task is shown, if one is waiting. */
  nextShown(now: number): number | undefined {
    const waiting = [...this.entries.values()].map((e) => e.shownFrom - now).filter((wait) => wait > 0);
    return waiting.length ? Math.min(...waiting) : undefined;
  }
}
