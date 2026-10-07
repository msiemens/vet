// What the welcome screen shows of the recent projects: the search over them, a project's initials
// and avatar colour, and its last-opened label.

import type { RecentProject } from "../../generated/RecentProject";
import { repoName } from "../../lib/paths.ts";
import { match, type Range } from "../../lib/speedSearch.ts";

/** How many avatar colours the palette (`--avatar-1` …) has. */
export const AVATAR_COLOURS = 6;

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

/** Which palette colour (1-based) a project gets: the same for the same path, whatever else is listed. */
export function avatarColour(path: string): number {
  let hash = 0;
  for (const char of path) hash = (Math.imul(hash, 31) + (char.codePointAt(0) ?? 0)) >>> 0;
  return (hash % AVATAR_COLOURS) + 1;
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

const weekday = new Intl.DateTimeFormat("en", { weekday: "short" });
const monthDay = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const monthDayYear = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });
const clock = new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" });

const startOfDay = (ms: number) => new Date(ms).setHours(0, 0, 0, 0);

/**
 * When a project was last opened, as short as a list row needs: "now" for the last minute, the time
 * of day today, the weekday within the last week, then the date (with the year when it is not this one).
 */
export function openedLabel(unixSeconds: number, nowMs = Date.now()): string {
  const at = unixSeconds * 1000;
  if (nowMs - at < 60_000) return "now";
  const days = Math.round((startOfDay(nowMs) - startOfDay(at)) / 86_400_000);
  if (days <= 0) return clock.format(at);
  if (days < 7) return weekday.format(at);
  return (new Date(at).getFullYear() === new Date(nowMs).getFullYear() ? monthDay : monthDayYear).format(at);
}
