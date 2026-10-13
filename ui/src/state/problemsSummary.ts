// What the footer item and the rail badge say of a changeset's problems. Missing analysis is never
// shown as zero (SPEC I6): a file no server has answered for counts as waiting or as not analysed,
// and a file whose problems cannot be told new from existing is named, not counted as either.

import { plural } from "../lib/plural.ts";
import type { Counts } from "./problemRows.ts";

/** Whether the answers for some files still compare with the base, say which problems are new, or could not. */
export type Origins = "known" | "pending" | "unknown";

/** What files' answers say together: pending while one is not compared yet, else unknown if one could not be. */
export function originsOf(answers: readonly { compared: boolean; originUnavailable?: unknown }[]): Origins {
  if (!answers.length || answers.some((a) => !a.compared)) return "pending";
  return answers.some((a) => a.originUnavailable) ? "unknown" : "known";
}

export interface SummaryInput {
  counts: Counts;
  /** Some file's comparison with the base is underway. */
  comparing: boolean;
  /** Files whose origins could not be told. */
  unknownFiles: number;
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
  /** What the badge counts, for a screen reader: only what is known to be new is called new. */
  badgeText: string;
  title: string;
}

export function summarize({ counts, comparing, unknownFiles, files, changed }: SummaryInput): Summary | undefined {
  if (files.total === 0) {
    return changed === 0
      ? undefined
      : {
          text: "Not analysed",
          badge: 0,
          badgeText: "",
          title: "No changed file has a language server to analyse it",
        };
  }
  const info = counts.info ? `, ${counts.info} information or hints` : "";
  const unknown = counts.unknown ? `, ${counts.unknown} of unknown origin` : "";
  const title = comparing
    ? `${counts.all} problems${info}; which are new is not known yet. Click to show Problems.`
    : `${counts.newOnes} new, ${counts.existing} existing${unknown}${info}. Click to show Problems.`;
  if (files.answered === 0) {
    return {
      text: "Waiting for the language server",
      icon: "busy",
      badge: 0,
      badgeText: "",
      title: "Problems appear as servers answer",
    };
  }
  const n = counts.headline;
  const badge = { badge: n, badgeText: `${n} ${comparing ? "problems" : "new"}` };
  if (files.answered < files.total) {
    return { text: `Analysing ${files.answered} of ${files.total} files`, icon: "busy", ...badge, title };
  }
  if (comparing) {
    return n === 0
      ? { text: "No problems", icon: "ok", ...badge, title }
      : { text: `${plural(n, "problem")} · comparing`, icon: counts.worst, ...badge, title };
  }
  const unsure = unknownFiles ? ` · origin unknown in ${plural(unknownFiles, "file")}` : "";
  if (n === 0) return { text: `No new problems${unsure}`, icon: unknownFiles ? undefined : "ok", ...badge, title };
  return { text: `${plural(n, "new problem")}${unsure}`, icon: counts.worst, ...badge, title };
}
