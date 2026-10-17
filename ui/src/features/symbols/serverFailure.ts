// A language server that does not start fails every answer about a symbol for the same reason. The
// panel says it once; the sections drop what repeats it and keep their own, different failures.

import type { Calls } from "../../generated/Calls";
import type { Gap } from "../../generated/Gap";
import type { Reachability } from "../../generated/Reachability";
import type { RemovalImpact } from "../../generated/RemovalImpact";
import type { Unavailable } from "../../generated/Unavailable";
import type { Usages } from "../../generated/Usages";
import type { Signature } from "../../state/symbols.svelte.ts";

/** What the answers about the symbol on show, as far as they are in. */
export interface Answers {
  signature?: Signature;
  calls?: Calls;
  usages?: Usages;
  reach?: Reachability;
  removal?: RemovalImpact;
}

/** `why` is the failure `shared` states: the same server failing the same way. */
export const repeats = (shared: Unavailable | undefined, why: Unavailable | null | undefined): boolean =>
  shared !== undefined &&
  !!why &&
  why.kind === "serverFailed" &&
  why.kind === shared.kind &&
  why.language === shared.language;

/** The gaps that are not `shared`'s failure again. */
export const unshared = (gaps: readonly Gap[], shared: Unavailable | undefined): Gap[] =>
  gaps.filter((g) => g.type !== "failed" || !repeats(shared, g.why));

/** Whether `shared` accounts for every one of `gaps`, of which there are some. */
export const allShared = (gaps: readonly Gap[], shared: Unavailable | undefined): boolean =>
  gaps.length > 0 && unshared(gaps, shared).length === 0;

/** The server failure found in the answers that are in: the first, there being one server per language. */
export function sharedFailure(answers: Answers): Unavailable | undefined {
  const gaps = [
    ...(answers.calls?.gaps ?? []),
    ...(answers.reach?.reach.type === "unknown" ? answers.reach.reach.gaps : []),
  ];
  const candidates = [
    answers.signature?.unavailable,
    ...gaps.map((g) => (g.type === "failed" ? g.why : undefined)),
    answers.usages?.gap,
    answers.removal?.gap,
  ];
  return candidates.find((u): u is Unavailable => u?.kind === "serverFailed");
}
