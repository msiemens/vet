import type { Thread } from "../generated/Thread";
import { lineRange } from "./lineRange.ts";

/** The text of the badge on an inferred thread: its lines moved, or were deleted. */
export function movedLabel(thread: Thread): string {
  return thread.anchor?.type === "inferred" && thread.anchor.strategy === "deleted" ? "line deleted" : "moved";
}

/** The tooltip of a "moved" badge: how the anchor was found and where it was recorded. */
export function movedTitle(thread: Thread): string {
  const anchor = thread.anchor;
  if (anchor?.type !== "inferred") return "";
  const was = thread.original
    ? `; was ${thread.original.path}:${lineRange(thread.original.start, thread.original.len)}`
    : "";
  if (anchor.strategy === "deleted") return `The commented lines were deleted; shown where they were${was}`;
  const how = anchor.strategy === "crossFileMove" ? "found in another file" : "found by its surrounding lines";
  return `Moved: ${how}, ${anchor.confidence} confidence${was}`;
}
