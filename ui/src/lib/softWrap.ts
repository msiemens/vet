/** Which files the review views wrap long lines of: prose only, every file, or none. */
export type SoftWrap = "prose" | "all" | "never";

export const SOFT_WRAPS: { id: SoftWrap; label: string }[] = [
  { id: "prose", label: "Prose" },
  { id: "all", label: "All files" },
  { id: "never", label: "Never" },
];
