/** How a changed file's diff is drawn: the deleted lines inline, or the two sides next to each other. */
export type Mode = "unified" | "split";

export const MODES: { id: Mode; label: string }[] = [
  { id: "unified", label: "Unified" },
  { id: "split", label: "Split" },
];

/** The file's own text with its changes marked, the third choice of a single file's view control. */
export const FILE_VIEW = { id: "file", label: "File" } as const;

/** What the review's views show: a diff in a mode, or a changed file's text. */
export type ViewMode = Mode | typeof FILE_VIEW.id;

/** The id of the command that shows the view `mode`. */
export const modeCommand = (mode: ViewMode) => `view.mode.${mode}`;
