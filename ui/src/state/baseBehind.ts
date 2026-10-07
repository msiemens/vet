import type { Base } from "../generated/Base";

/** How far the remote's branch has moved on from the chosen base branch, e.g. "11 behind origin/master". */
export function behindText(base: Base | undefined): string | undefined {
  if (!base?.behind || base.choice.type !== "branch") return undefined;
  return `${base.behind} behind origin/${base.choice.name}`;
}
