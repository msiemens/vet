// What a thread's brief card (`briefCard.ts`) shows of its messages, worked out apart from the DOM.

/** How many lines of a message the brief card shows before "Show all". */
export const BRIEF_LINES = 4;

/** A message this long is cut by the lines of a narrow card, whatever its own line breaks. */
const LONG_MESSAGE = 240;

/** Whether a message is cut short in a brief card, so it offers "Show all". */
export function needsShowAll(text: string): boolean {
  return text.length > LONG_MESSAGE || text.split("\n").length > BRIEF_LINES;
}

/** The first message and the last, and how many lie between them; a thread of two or fewer shows all. */
export function briefMessages<T>(messages: readonly T[]): { shown: T[]; between: number } {
  if (messages.length <= 2) return { shown: [...messages], between: 0 };
  return { shown: [messages[0], messages[messages.length - 1]], between: messages.length - 2 };
}
