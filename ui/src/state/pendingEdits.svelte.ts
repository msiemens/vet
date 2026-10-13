// Whether edits of the working tree wait to settle into a snapshot, which the header's dot shows.

import { session } from "./session.svelte";

class PendingEdits {
  /** Edits were seen and the snapshot that makes them a version is not taken yet. */
  waiting = $state(false);

  constructor() {
    session.onConnect((backend) =>
      backend.onEvent((event) => {
        if (event.project === backend.project && event.type === "editsPending") this.waiting = event.pending;
      }),
    );
    session.onResync(() => (this.waiting = false));
    session.onClose(() => (this.waiting = false));
  }
}

export const pendingEdits = new PendingEdits();
