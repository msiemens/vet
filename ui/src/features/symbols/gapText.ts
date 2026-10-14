import type { Gap } from "../../generated/Gap";

/** What a derived result lacks, in words. */
export function gapText(gap: Gap): string {
  switch (gap.type) {
    case "failed":
      return gap.what === null ? gap.why.message : `${gap.what}: ${gap.why.message}`;
    case "tooManyCallers":
      return `more than ${gap.max} callers`;
    case "tooDeep":
      return `callers more than ${gap.maxDepth} levels up were not followed`;
  }
}
