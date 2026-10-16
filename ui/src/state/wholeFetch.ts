// When the whole-changeset view must be read again. The changeset moves on only in a settle, a
// target change or a resync; a mark names the revision already held and is written into the view,
// so a burst of marks costs no whole-changeset fetch.

/** The revision an event names, or none when it names none (a settle's target move, a resync). */
export type EventRevision = number | undefined;

/**
 * Whether an event at `event` makes a whole-changeset view held at `held` stale. An event that names
 * no revision moves the changeset regardless; an event at the held revision (a mark) reads nothing;
 * an event at an older revision says nothing of the changeset held.
 */
export function refetchesWhole(event: EventRevision, held: EventRevision): boolean {
  if (event === undefined) return true;
  return held === undefined || event > held;
}
