// The backend's background tasks (scanning, diffing, restoring, servers starting and indexing,
// the symbol analysis) that the footer shows. The backend throttles a task's updates; this holds
// the running ones and hides a step that is over in a blink.

import type { Task } from "../generated/Task";
import { Debounced } from "../lib/debounced";
import type { Backend } from "./backend";
import { opened } from "./opened.svelte";
import { session } from "./session.svelte";
import { TaskBoard } from "./taskBoard";

/** How long a task runs before it is shown, so a rescan after an edit does not flash. */
const SHOWN_AFTER_MS = 400;

class ProgressStore {
  /** The running tasks to show, oldest first. */
  tasks = $state.raw<Task[]>([]);
  /** The failed tasks that stay shown until the next success of their kind. */
  failed = $state.raw<Task[]>([]);
  private board = new TaskBoard();
  private timer = new Debounced();

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        // Tasks announced before the open request was answered are listed by `onOpen`.
        if (event.type !== "task" || event.project !== backend.project) return;
        this.board.apply(event, performance.now(), this.delay());
        this.publish();
      }),
    );
    session.onOpen((backend) => this.list(backend));
    session.onResync((backend) => {
      if (session.repo !== undefined) this.list(backend);
    });
    session.onClose(() => {
      this.board.clear();
      this.publish();
    });
  }

  /** Replaces the tasks with those the backend has running. */
  private list(backend: Backend) {
    void backend.tasks().then(
      (tasks) => {
        this.board.replace(tasks, performance.now(), this.delay());
        this.publish();
      },
      (error: unknown) => console.error(error),
    );
  }

  /** While the project opens the footer is all there is to see, so its tasks show at once. */
  private delay(): number {
    return opened.session?.layout ? SHOWN_AFTER_MS : 0;
  }

  private publish() {
    this.timer.cancel();
    const now = performance.now();
    this.tasks = this.board.shown(now);
    this.failed = this.board.failed();
    const wait = this.board.nextShown(now);
    if (wait !== undefined) this.timer.schedule(wait, () => this.publish());
  }
}

export const progress = new ProgressStore();
