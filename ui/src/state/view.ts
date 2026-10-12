// The views requests about the changeset's files can name (see `generated/View`).

import type { View } from "../generated/View";

/** The changeset itself, which a request that names no other view reads. */
export const CHANGESET: View = { type: "changeset" };

/** An interdiff of `from` and the later `to`. */
export const interdiffView = (from: number, to: number): View => ({ type: "interdiff", from, to });

/** What the commit at `index` of the changeset changed. */
export const commitView = (index: number): View => ({ type: "commit", index });

export function sameView(a: View, b: View): boolean {
  if (a.type === "interdiff") return b.type === "interdiff" && a.from === b.from && a.to === b.to;
  if (a.type === "commit") return b.type === "commit" && a.index === b.index;
  return b.type === "changeset";
}
