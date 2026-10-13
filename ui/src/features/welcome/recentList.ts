// What the welcome screen shows of the recent projects: the search over them and a project's initials.

import type { RecentProject } from "../../generated/RecentProject";
import { repoName } from "../../lib/paths.ts";
import { match, type Range } from "../../lib/speedSearch.ts";

export interface RecentMatch {
  project: RecentProject;
  name: string;
  /** Where the query matched in `name` and in the path, for highlighting. */
  nameRanges: Range[];
  pathRanges: Range[];
}

/** Two letters for an avatar: the first letters of the first two words of a name, else its first two characters. */
export function initials(name: string): string {
  const words = name.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase();
  return Array.from(words[0] ?? name)
    .slice(0, 2)
    .join("")
    .toUpperCase();
}

function merge(ranges: Range[]): Range[] {
  const sorted = [...ranges].sort((a, b) => a[0] - b[0]);
  const merged: Range[] = [];
  for (const [start, end] of sorted) {
    const last = merged.at(-1);
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else merged.push([start, end]);
  }
  return merged;
}

/**
 * The projects whose name or path matches every word of `query` (speed search matching: ignoring
 * case, by word starts or as a run), in their order. An empty query matches all.
 */
export function searchRecent(projects: RecentProject[], query: string): RecentMatch[] {
  const terms = query.split(/\s+/).filter(Boolean);
  const out: RecentMatch[] = [];
  for (const project of projects) {
    const name = repoName(project.path);
    const nameRanges: Range[] = [];
    const pathRanges: Range[] = [];
    let all = true;
    for (const term of terms) {
      const inName = match(term, name);
      const inPath = match(term, project.path);
      if (!inName && !inPath) {
        all = false;
        break;
      }
      if (inName) nameRanges.push(...inName.ranges);
      if (inPath) pathRanges.push(...inPath.ranges);
    }
    if (all) out.push({ project, name, nameRanges: merge(nameRanges), pathRanges: merge(pathRanges) });
  }
  return out;
}
