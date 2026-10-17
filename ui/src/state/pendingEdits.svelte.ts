// Whether edits of the working tree wait to settle into a snapshot, which the header's dot shows.

import { session } from "./session.svelte";

class PendingEdits {
  /** Edits were seen and the snapshot that makes them a version is not taken yet. */
  waiting = $state(false);

  constructor() {
    session.onConnect((backend) => {
      backend.onEvent((event) => {
        if (event.type === "editsPending") this.waiting = event.pending;
      });
      // What was missed while the link was down is not known; opening the project again tells whether edits wait.
      backend.onLinkState((state) => {
        if (state !== "connected") this.waiting = false;
      });
    });
    session.onClose(() => (this.waiting = false));
  }
}

export const pendingEdits = new PendingEdits();
