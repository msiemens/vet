// The window's own state that the template binds and the commands act on: the review screen's
// controller while it is mounted, the dialog on show, and what the host says of the build. The screen
// and the tool windows are the chrome's `shell`.

import type { ReviewController } from "../features/review";
import type { Page } from "../features/settings";
import { IntentQueue } from "./intentQueue";

type Dialog = "settings" | "about" | "finish" | "pullRequest";

class AppState {
  /** The review screen's controller, while its screen is mounted. */
  controller = $state.raw<ReviewController>();
  /** What opening the review screen asked of it, run once its controller is ready. */
  private intents = new IntentQueue<ReviewController>();
  /** The dialog on show, if one is. */
  dialog = $state<Dialog>();
  /** The page Settings shows. */
  settingsPage = $state<Page>("general");
  /** The changeset switcher's popover is open. */
  switcherOpen = $state(false);
  /** The app version the host reports; none in a plain browser. */
  version = $state<string>();
  /** The host's name; none in a plain browser. */
  host = $state<string>();

  /**
   * Runs `act` with the review screen's controller: now while it is mounted, else once opening the
   * screen has built it. A caller that opens the screen first leaves the operation here, so it is
   * not dropped while the controller is still coming up.
   */
  whenReview(act: (controller: ReviewController) => void) {
    this.intents.when(act, this.controller);
  }

  /** The review screen built its controller: what opening it asked for runs. */
  setController(controller: ReviewController) {
    this.controller = controller;
    this.intents.ready(controller);
  }

  /** The review screen was torn down; what it still had queued goes with it. */
  clearController() {
    this.controller = undefined;
    this.intents.clear();
  }

  /** Shows Settings on `page`, or the About dialog. */
  showDialog(dialog: "settings" | "about", page: Page = "general") {
    this.settingsPage = page;
    this.dialog = dialog;
  }

  closeDialog() {
    this.dialog = undefined;
  }
}

export const appState = new AppState();
