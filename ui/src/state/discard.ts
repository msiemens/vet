// The question asked before text typed into comment boxes and not added as a comment would be lost:
// by switching to another changeset, closing the project or quitting.

/** What the dialog says under its title. */
export const DISCARD_BODY = "You haven't added the comment you're writing. It will be lost.";

/** The dialog's title for `count` boxes that hold text. */
export const discardTitle = (count: number): string =>
  count === 1 ? "Discard your comment?" : `Discard ${count} comments?`;

/**
 * Runs `go` when no box holds text, at once, else after `ask` answers that the text may be discarded;
 * a kept text runs nothing.
 */
export async function whenDiscarded(
  count: number,
  ask: (count: number) => Promise<boolean>,
  go: () => Promise<void> | void,
): Promise<void> {
  if (count > 0 && !(await ask(count))) return;
  await go();
}
