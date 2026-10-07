import type { Base } from "../generated/Base";

/** How far the remote's branch has moved on from the base's target, e.g. "11 behind origin/master". */
export function behindText(base: Base | undefined): string | undefined {
  if (!base?.behind || base.choice.type !== "mergeBase") return undefined;
  const { reference } = base.choice;
  return `${base.behind} behind ${reference.startsWith("origin/") ? reference : `origin/${reference}`}`;
}
