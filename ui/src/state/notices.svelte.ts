// The notice shown to the viewer: a short message about something that did not go through, or a
// result worth a line. One notice is on show; a newer one replaces it (a notice that says what the
// one on show says does not restart it), and the viewer closes it or it closes itself.

import { failureNotice } from "../lib/notice";

class Notices {
  text = $state<string>();

  show(text: string) {
    this.text = text;
  }

  /** Tells the viewer a request failed while `what` was being done, unless it was given up on. */
  failed(what: string, error: unknown) {
    this.tell(failureNotice(what, error));
  }

  /** Tells the viewer a request failed, in the words of the failure alone. */
  report(error: unknown) {
    this.tell(failureNotice(undefined, error));
  }

  private tell(text: string | undefined) {
    if (text !== undefined) this.show(text);
  }

  close() {
    this.text = undefined;
  }
}

export const notices = new Notices();
