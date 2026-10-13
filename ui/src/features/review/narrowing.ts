// How a review narrowed to a filter other than the project's is worded, from what differs between the two.

import type { ReviewFilter } from "../../generated/ReviewFilter";

/**
 * What `narrowing` shows that the project's filter does not, as a chip words it: "Only src/api" for
 * folders, "Also generated" for rules that are lifted, "Only src/api + generated" for both.
 */
export function narrowingLabel(project: ReviewFilter, narrowing: ReviewFilter): string {
  const only =
    narrowing.roots.length && narrowing.roots.join("\0") !== project.roots.join("\0") ? narrowing.roots.join(", ") : "";
  const also: string[] = [];
  if (!narrowing.roots.length && project.roots.length) also.push("outside roots");
  if (!narrowing.include.length && project.include.length) also.push("not included");
  for (const pattern of project.exclude) if (!narrowing.exclude.includes(pattern)) also.push(`matching ${pattern}`);
  if (narrowing.reviewGenerated && !project.reviewGenerated) also.push("generated");
  if (!narrowing.hideBinary && project.hideBinary) also.push("binary");
  if (only && also.length) return `Only ${only} + ${also.join(", ")}`;
  if (only) return `Only ${only}`;
  if (also.length) return `Also ${also.join(", ")}`;
  return "Only part of the project";
}
