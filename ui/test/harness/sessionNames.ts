// The names of the scripted sessions. The child that runs one and the test that judges it agree on
// these; `sessions.ts` is typed against them so the two cannot drift.

export const sessionNames = [
  "changeset",
  "push",
  "range",
  "commits",
  "pinned",
  "stepping",
  "otherWindow",
  "reconnect",
  "listings",
  "queue",
  "preferences",
  "rapid",
] as const;

export type SessionName = (typeof sessionNames)[number];
