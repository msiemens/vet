// The window's own state that the template binds and the commands act on: the review screen's
// controller while it is mounted, the dialog on show, and what the host says of the build. The screen
// and the tool windows are the chrome's `shell`.

import type { ReviewController } from "../features/review";
import type { Page } from "../features/settings";

type Dialog = "settings" | "about" | "finish" | "pullRequest";

class AppState {
  /** The review screen's controller, while its screen is mounted. */
  controller = $state.raw<ReviewController>();
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
