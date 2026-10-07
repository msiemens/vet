// What the header says about the base and the review version, from the facts the backend sent.

import type { Base } from "../../generated/Base";
import type { BaseChoice } from "../../generated/BaseChoice";
import type { BranchInfo } from "../../generated/BranchInfo";
import { type MenuItem, separator } from "../../lib/ui/menuItem.ts";
import { relativeTime } from "./relativeTime.ts";

/** How the choice came about, for the tooltip. `branch` is the working tree's. */
export function baseDescription(choice: BaseChoice, branch: string | undefined): string {
  switch (choice.type) {
    case "mergeBase":
      return choice.reference === branch
        ? `Tip of ${branch}`
        : `Where ${branch ?? "HEAD"} branched off ${choice.reference}`;
    case "commit":
      return "A commit";
  }
}

/** Full hash, subject, author and age of the base commit, under what the choice is. */
export function baseTooltip(base: Base, branch: string | undefined, nowMs = Date.now()): string {
  const c = base.commit;
  return [baseDescription(base.choice, branch), c.id, c.subject, `${c.author} · ${relativeTime(c.time, nowMs)}`].join(
    "\n",
  );
}

export function versionTitle(version: number, updatedAt: number, nowMs = Date.now()): string {
  return `Version ${version} · updated ${relativeTime(updatedAt, nowMs)}`;
}

/** The current branch first, the default one next, then the rest in the order listed. */
export function orderBranches(branches: BranchInfo[]): BranchInfo[] {
  return [...branches].sort((a, b) => Number(b.current) - Number(a.current) || Number(b.default) - Number(a.default));
}

/**
 * The base menu: each branch as the target to compare against, and a commit to pick. A changeset's
 * own branch (`own`) is not offered as its own target. Item ids are read back by `parseBaseItem`.
 */
export function baseMenuItems(
  branches: BranchInfo[],
  choice: BaseChoice | undefined,
  commitsListed: boolean,
  /** How far the chosen target's remote is ahead, shown next to that branch. */
  behind?: string,
  own?: string,
): MenuItem[] {
  const offered = orderBranches(branches).filter((b) => b.name !== own);
  return [
    ...offered.map((b) => {
      const chosen = choice?.type === "mergeBase" && choice.reference === b.name;
      const detail = [b.current ? "current" : b.default ? "default" : undefined, chosen ? behind : undefined]
        .filter(Boolean)
        .join(" · ");
      return { id: `branch:${b.name}`, label: b.name, detail: detail || undefined, checked: chosen };
    }),
    ...(offered.length ? [separator] : []),
    { id: "commit", label: "Pick a commit…", enabled: commitsListed, checked: choice?.type === "commit" },
  ];
}

/** What a chosen item of `baseMenuItems` asks for: a base, or the commit picker. */
export function parseBaseItem(id: string): BaseChoice | "commits" | undefined {
  if (id === "commit") return "commits";
  if (id.startsWith("branch:")) return { type: "mergeBase", reference: id.slice("branch:".length) };
  return undefined;
}
