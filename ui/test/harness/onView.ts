import type { ChangesetSession } from "../../src/state/changesetSession.svelte.ts";
import { opened } from "../../src/state/opened.svelte.ts";

/** The session of the changeset on view, for a step that acts on it; throws when none is. */
export function onView(): ChangesetSession {
  const session = opened.session;
  if (!session) throw new Error("no changeset is on view");
  return session;
}
