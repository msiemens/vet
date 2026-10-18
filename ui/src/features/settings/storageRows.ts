// How the app-wide Storage page lists the projects: what a project takes in all, largest first.

import type { ProjectStorage } from "../../generated/ProjectStorage.ts";
import { compareText } from "../../lib/format.ts";

type Figures = Pick<ProjectStorage, "reviewData" | "sourceTrees" | "semanticCache">;

/** Everything the project takes on disk. */
export const projectTotal = (p: Figures): number => p.reviewData + p.sourceTrees + p.semanticCache;

/** The projects by what they take, the largest first; equal ones by root. */
export function largestFirst(projects: readonly ProjectStorage[]): ProjectStorage[] {
  return [...projects].sort((a, b) => projectTotal(b) - projectTotal(a) || compareText(a.root, b.root));
}
