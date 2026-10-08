/** How a changed file's diff is drawn: the deleted lines inline, or the two sides next to each other. */
export type Mode = "unified" | "split";

export const MODES: { id: Mode; label: string }[] = [
  { id: "unified", label: "Unified" },
  { id: "split", label: "Split" },
];

/** The file's own text with its changes marked, the third choice of a single file's view control. */
export const FILE_VIEW = { id: "file", label: "File" } as const;
