import type { Thread } from "../generated/Thread";
import { lineRange } from "./lineRange.ts";

/** The tooltip of a "moved" badge: how the anchor was found and where it was recorded. */
export function movedTitle(thread: Thread): string {
  const anchor = thread.anchor;
  if (anchor?.type !== "inferred") return "";
  const how = anchor.strategy === "crossFileMove" ? "found in another file" : "found by its surrounding lines";
  const was = thread.original
    ? `; was ${thread.original.path}:${lineRange(thread.original.start, thread.original.len)}`
    : "";
  return `Moved: ${how}, ${anchor.confidence} confidence${was}`;
}
