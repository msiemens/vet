import { readStored, writeStored } from "./storage.ts";

/** Which files the review views wrap long lines of: prose only, every file, or none. */
export type SoftWrap = "prose" | "all" | "never";

export const SOFT_WRAPS: { id: SoftWrap; label: string }[] = [
  { id: "prose", label: "Prose" },
  { id: "all", label: "All files" },
  { id: "never", label: "Never" },
];

const KEY = "vet.softWrap";

export function storeSoftWrap(wrap: SoftWrap) {
  writeStored(KEY, wrap);
}

export function storedSoftWrap(): SoftWrap {
  const value = readStored(KEY);
  return value === "all" || value === "never" ? value : "prose";
}
