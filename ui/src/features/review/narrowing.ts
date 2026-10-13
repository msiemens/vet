// How a review narrowed to a filter other than the project's is worded, from what differs between the two.

import type { ReviewFilter } from "../../generated/ReviewFilter";

/** What `narrowing` shows that the project's filter does not, as a chip words it: "Only src/api", "Only generated". */
export function narrowingLabel(project: ReviewFilter, narrowing: ReviewFilter): string {
  const parts: string[] = [];
  if (narrowing.roots.length && narrowing.roots.join("\0") !== project.roots.join("\0")) {
    parts.push(narrowing.roots.join(", "));
  } else if (!narrowing.roots.length && project.roots.length) parts.push("outside roots");
  if (!narrowing.include.length && project.include.length) parts.push("not included");
  for (const pattern of project.exclude) if (!narrowing.exclude.includes(pattern)) parts.push(`matching ${pattern}`);
  if (narrowing.reviewGenerated && !project.reviewGenerated) parts.push("generated");
  if (!narrowing.hideBinary && project.hideBinary) parts.push("binary");
  return `Only ${parts.join(", ") || "part of the project"}`;
}
