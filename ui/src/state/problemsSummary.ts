// What the footer item and the rail badge say of a changeset's problems. Missing analysis is never
// shown as zero (SPEC I6): a file no server has answered for counts as waiting or as not analysed.

import type { Counts } from "./problemRows.ts";

export type Origins = "known" | "pending" | "unknown";

export interface SummaryInput {
  counts: Counts;
  origins: Origins;
  /** Changed files a language server could analyse, and how many of them have answered (or failed to). */
  files: { total: number; answered: number };
  /** Changed files at all, whether a server serves them or not. */
  changed: number;
}

export interface Summary {
  text: string;
  /** Which icon leads the text. */
  icon?: "error" | "warning" | "ok" | "busy";
  /** The number on the rail's icon; none while nothing has answered. */
  badge: number;
  title: string;
}

export function summarize({ counts, origins, files, changed }: SummaryInput): Summary | undefined {
  if (files.total === 0) {
    return changed === 0
      ? undefined
      : { text: "Not analysed", badge: 0, title: "No changed file has a language server to analyse it" };
  }
  const title = `${counts.newOnes} new, ${counts.existing} existing. Click to show Problems.`;
  if (files.answered === 0) {
    return {
      text: "Waiting for the language server",
      icon: "busy",
      badge: 0,
      title: "Problems appear as servers answer",
    };
  }
  const known = origins === "known";
  const n = counts.headline;
  const badge = n;
  if (files.answered < files.total) {
    return { text: `Analysing ${files.answered} of ${files.total} files`, icon: "busy", badge, title };
  }
  if (known) {
    return n === 0
      ? { text: "No new problems", icon: "ok", badge, title }
      : { text: `${n} new problem${n === 1 ? "" : "s"}`, icon: counts.worst, badge, title };
  }
  if (n === 0) return { text: "No problems", icon: "ok", badge, title };
  const comparing = origins === "pending" ? " · comparing" : "";
  return { text: `${n} problem${n === 1 ? "" : "s"}${comparing}`, icon: counts.worst, badge, title };
}
