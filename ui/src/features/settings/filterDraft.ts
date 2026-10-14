// The review filter page's drafts: what the viewer has typed into the pattern lists that is not
// saved yet, and the words for what the backend counted for it. Globs are the backend's to
// judge (SPEC I5); nothing here reads one.

import type { PatternMatch } from "../../generated/PatternMatch";
import { plural } from "../../lib/format.ts";

/** What is typed into a pattern list: edits to rows by index, and the text of the add field. */
export interface PatternDraft {
  edits: Partial<Record<number, string>>;
  adding: string;
}

/** The patterns the backend is asked about: the list with the edits applied, then the add field's text if it holds any. */
export function draftPatterns(patterns: readonly string[], draft: PatternDraft): string[] {
  const added = draft.adding.trim();
  return [...patterns.map((pattern, i) => draft.edits[i] ?? pattern), ...(added ? [added] : [])];
}

/** The list a row's edit commits: the row replaced, or dropped when it was emptied. */
export function withEdit(patterns: readonly string[], index: number, text: string): string[] {
  const edited = text.trim();
  return patterns.flatMap((pattern, i) => (i !== index ? [pattern] : edited ? [edited] : []));
}

/** A typed root as the filter keeps it: a folder without a slash at either end. */
export function rootPath(text: string): string {
  return text.trim().replace(/^\/+|\/+$/g, "");
}

/** What a pattern matches, as a cell says it. */
export function matchText(match: Pick<PatternMatch, "files" | "lines">): string {
  return `${plural(match.files, "file")} · ${plural(match.lines, "line")}`;
}
