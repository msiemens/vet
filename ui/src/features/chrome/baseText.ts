// What the header says about the base and the review version, from the facts the backend sent.

import type { Base } from "../../generated/Base";
import type { BaseChoice } from "../../generated/BaseChoice";
import type { BranchInfo } from "../../generated/BranchInfo";
import { type MenuItem, separator } from "../../lib/ui/menuItem.ts";
import { relativeTime } from "./relativeTime.ts";

/** How the choice came about, for the tooltip. `branch` is the working tree's. */
export function baseDescription(choice: BaseChoice, branch: string | undefined): string {
  switch (choice.type) {
    case "branch":
      return `Tip of ${choice.name}`;
    case "mergeBase":
      return `Where ${branch ?? "HEAD"} branched off ${choice.reference}`;
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
 * The base menu: each branch's latest, where the working tree's branch branched off another, and a
 * commit to pick. Item ids are read back by `parseBaseItem`.
 */
export function baseMenuItems(
  branches: BranchInfo[],
  choice: BaseChoice | undefined,
  commitsListed: boolean,
): MenuItem[] {
  const ordered = orderBranches(branches);
  const current = ordered.find((b) => b.current);
  const others = ordered.filter((b) => !b.current);
  return [
    ...ordered.map((b) => ({
      id: `branch:${b.name}`,
      label: `${b.name} (latest)`,
      detail: b.current ? "current" : b.default ? "default" : undefined,
      checked: choice?.type === "branch" && choice.name === b.name,
    })),
    ...(ordered.length ? [separator] : []),
    {
      id: "mergeBase",
      label: `Where ${current?.name ?? "HEAD"} branched off`,
      enabled: others.length > 0,
      checked: choice?.type === "mergeBase",
      children: others.map((b) => ({
        id: `mb:${b.name}`,
        label: b.name,
        detail: b.default ? "default" : undefined,
        checked: choice?.type === "mergeBase" && choice.reference === b.name,
      })),
    },
    { id: "commit", label: "Pick a commit…", enabled: commitsListed, checked: choice?.type === "commit" },
  ];
}

/** What a chosen item of `baseMenuItems` asks for: a base, or the commit picker. */
export function parseBaseItem(id: string): BaseChoice | "commits" | undefined {
  if (id === "commit") return "commits";
  if (id.startsWith("branch:")) return { type: "branch", name: id.slice("branch:".length) };
  if (id.startsWith("mb:")) return { type: "mergeBase", reference: id.slice("mb:".length) };
  return undefined;
}
