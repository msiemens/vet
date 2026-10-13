// The three options of a text search, as the find bar and Find in Files offer them.

export interface SearchOptionToggle {
  key: "matchCase" | "wholeWord" | "regex";
  label: string;
  /** What the tooltip calls it. */
  name: string;
  /** The key that flips it with Option held, read by its place since Option types a letter. */
  code: string;
  /** The command that carries its binding. */
  id: string;
}

export const SEARCH_OPTION_TOGGLES: readonly SearchOptionToggle[] = [
  { key: "matchCase", label: "Aa", name: "Match case", code: "KeyC", id: "find.matchCase" },
  { key: "wholeWord", label: "ab", name: "Whole word", code: "KeyW", id: "find.wholeWord" },
  { key: "regex", label: ".*", name: "Regular expression", code: "KeyR", id: "find.regex" },
];
