// How the viewer hears what a notification is about. The backend sends what happened; these are
// the sentences for it.

import type { Notification } from "../../generated/Notification";
import { platformName } from "../../lib/platformName.ts";

/** The message for the card in the window's corner and the footer's bell list. */
export function notificationText(notification: Notification): string {
  switch (notification.type) {
    case "threadDeleted":
      return (
        `Thread on ${notification.place} was deleted on ${platformName(notification.platform)}.` +
        (notification.dropped.length ? " Your draft reply was removed." : "")
      );
    case "voteNotCast":
      return `The vote was not cast: ${notification.problem}`;
    case "commentsNotPosted":
      return `${counted(notification.notPosted.length)} not posted: ${notification.problem}`;
    case "serverDied":
      return notification.outcome === "restarted"
        ? `${notification.name} stopped unexpectedly and was restarted.`
        : `${notification.name} stopped unexpectedly again and stays stopped. Restart it from the Help menu.`;
    case "syncStopped":
      return notification.reason === "signedOut"
        ? `Not signed in to ${notification.host}: comments are not synced.`
        : `The token for ${notification.host} is no longer accepted: comments are not synced.`;
  }
}

/** What "Copy text" copies, when there is something worth keeping from the notification. */
export function notificationCopyText(notification: Notification): string | null {
  if (notification.type === "threadDeleted") return kept(notification.dropped);
  if (notification.type === "commentsNotPosted") return kept(notification.notPosted);
  return null;
}

function counted(count: number): string {
  return count <= 1 ? "A comment was" : `${count} comments were`;
}

function kept(texts: string[]): string | null {
  return texts.length ? texts.join("\n\n") : null;
}
