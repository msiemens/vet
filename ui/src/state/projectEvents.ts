// Which events belong to the open project.

import type { Event as ServerEvent } from "../generated/Event";

/** The events that belong to the backend, not to a session. */
export type ProcessEvent = Extract<
  ServerEvent,
  {
    type:
      | "credentialsNeeded"
      | "accountsChanged"
      | "recentProjectsChanged"
      | "projectRemoved"
      | "storageChanged"
      | "settingsChanged"
      | "preferenceChanged";
  }
>;

/** The events of a session. */
export type SessionEvent = Exclude<ServerEvent, ProcessEvent>;

const PROCESS_EVENTS: ReadonlySet<string> = new Set<ProcessEvent["type"]>([
  "credentialsNeeded",
  "accountsChanged",
  "recentProjectsChanged",
  "projectRemoved",
  "storageChanged",
  "settingsChanged",
  "preferenceChanged",
]);

export const isProcessEvent = (event: ServerEvent): event is ProcessEvent => PROCESS_EVENTS.has(event.type);

/** Calls `listener` with the events of the project open now (`project()`): none of another project, none while none is open, none of the backend's own. Returns the unsubscribe function. */
export function projectEvents(
  source: (listener: (event: ServerEvent) => void) => () => void,
  project: () => number | undefined,
  listener: (event: SessionEvent) => void,
): () => void {
  return source((event) => {
    if (!isProcessEvent(event) && event.project === project()) listener(event);
  });
}
