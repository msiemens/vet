/** How a changed file is drawn: the deleted lines inline, or the two sides next to each other. */
export type Mode = "unified" | "split";

export const MODES: { id: Mode; label: string }[] = [
  { id: "unified", label: "Unified" },
  { id: "split", label: "Side by side" },
];
