// The backend's background tasks (scanning, diffing, restoring, servers starting and indexing,
// the symbol analysis) that the footer shows. The backend throttles a task's updates; this holds
// the running ones and hides a step that is over in a blink.

import type { Task } from "../generated/Task";
import { changeset } from "./changeset.svelte";
import { session } from "./session.svelte";
import { TaskBoard } from "./taskBoard";

/** How long a task runs before it is shown, so a rescan after an edit does not flash. */
const SHOWN_AFTER_MS = 400;

class ProgressStore {
  /** The running tasks to show, oldest first. */
  tasks = $state.raw<Task[]>([]);
  private board = new TaskBoard();
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        // Tasks announced before the open request was answered are listed by `onOpen`.
        if (event.type !== "task" || event.project !== backend.project) return;
        this.board.apply(event, performance.now(), this.delay());
        this.publish();
      }),
    );
    session.onOpen((backend) => {
      void backend.tasks().then(
        (tasks) => {
          this.board.replace(tasks, performance.now(), this.delay());
          this.publish();
        },
        (error: unknown) => console.error(error),
      );
    });
    session.onClose(() => {
      this.board.clear();
      this.publish();
    });
  }

  /** While the project opens the footer is all there is to see, so its tasks show at once. */
  private delay(): number {
    return changeset.manifest ? SHOWN_AFTER_MS : 0;
  }

  private publish() {
    clearTimeout(this.timer);
    const now = performance.now();
    this.tasks = this.board.shown(now);
    const wait = this.board.nextShown(now);
    if (wait !== undefined) this.timer = setTimeout(() => this.publish(), wait);
  }
}

export const progress = new ProgressStore();
