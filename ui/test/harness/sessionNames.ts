// The names of the scripted sessions. The child that runs one and the test that judges it agree on
// these; `sessions.ts` is typed against them so the two cannot drift.

export const sessionNames = [
  "changeset",
  "push",
  "range",
  "whole",
  "commits",
  "commitsTool",
  "pinned",
  "stepping",
  "otherWindow",
  "reconnect",
  "listings",
  "queue",
  "pipelines",
  "preferences",
  "rapid",
  "projectDialogs",
] as const;

export type SessionName = (typeof sessionNames)[number];

/** The sessions that start on the queue, with the review screen not shown. */
export const startOnQueue = new Set<SessionName>(["queue"]);
