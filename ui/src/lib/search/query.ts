// A text query as the find bar and Find in Files take it: a literal or a regular expression, with
// match case and whole word. The browser's regular expressions run it here; Find in Files runs the
// same options in the backend.

/** What is looked for, and how. */
export interface SearchText {
  text: string;
  matchCase: boolean;
  wholeWord: boolean;
  regex: boolean;
}

/** A range of one line's text, in UTF-16 units. */
export interface Hit {
  from: number;
  to: number;
}

/** A query ready to run: nothing to look for (`none`), a pattern the engine rejects, or the expression. */
export type Compiled = { type: "none" } | { type: "error"; message: string } | { type: "ok"; re: RegExp };

const SPECIAL = /[.*+?^${}()|[\]\\]/g;

/** The engine's complaint without the pattern it repeats. */
function reason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/^Invalid regular expression: \/.*\/[a-z]*: /, "");
}

/** A character of a word as Unicode has it, which is what the backend's `\b` is made of. */
const WORD = "[\\p{Alphabetic}\\p{M}\\p{Nd}\\p{Pc}\\p{Join_Control}]";
/** `\b` for Unicode text: the browser's own knows ASCII letters only, so `über` would not be a word. */
const BOUNDARY = `(?:(?<=${WORD})(?!${WORD})|(?<!${WORD})(?=${WORD}))`;

export function compile(query: SearchText): Compiled {
  if (!query.text) return { type: "none" };
  let source = query.regex ? query.text : query.text.replace(SPECIAL, "\\$&");
  if (query.wholeWord) source = `${BOUNDARY}(?:${source})${BOUNDARY}`;
  try {
    return { type: "ok", re: new RegExp(source, query.matchCase ? "gu" : "giu") };
  } catch (error) {
    return { type: "error", message: reason(error) };
  }
}

/** The non-empty matches in one line of text, in order. */
export function lineHits(re: RegExp, text: string): Hit[] {
  const hits: Hit[] = [];
  re.lastIndex = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m[0] === "") re.lastIndex++;
    else hits.push({ from: m.index, to: m.index + m[0].length });
  }
  return hits;
}
