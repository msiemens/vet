import type { WatchDegraded } from "../../generated/WatchDegraded";

/** What the viewer is told when the backend polls the tree instead of watching it. */
export const degradedMessage: Record<WatchDegraded, string> = {
  mountedWindowsDrive:
    "This repository is on a mounted Windows drive, where file events do not reach Linux. Vet checks for changes periodically, so edits show up late.",
  watchLimit:
    "The system's file watch limit is reached. Vet checks for changes periodically, so edits show up late; raising fs.inotify.max_user_watches restores live updates.",
};
