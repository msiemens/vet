// Which view of the changeset is asked for, and how a manifest read settles on one. The session holds
// one choice; a read that finds the choice cannot be shown corrects it only if the choice is still
// what that read was asked for.

import type { Lens } from "../generated/Lens";
import type { View } from "../generated/View";
import { isKind } from "../lib/requestError.ts";
import { commitToShow } from "./commitLens.ts";
import { type Range, normalised, rangeView } from "./timelineRange.ts";
import { commitView } from "./view.ts";

export interface ViewChoice {
  /** The versions compared instead of the changeset. */
  readonly range?: Range;
  /** The commit shown while the changeset is reviewed by commit; a range takes its place. */
  readonly commit?: number;
  /** The commit that was on view last, which by commit shows again. */
  readonly last: number;
  /** While only the files that had hunks seen against an old base are shown: their paths. */
  readonly limitedTo?: ReadonlySet<string>;
}

export const FIRST_CHOICE: ViewChoice = { last: 0 };

/** What one read asks for: a range, else a commit, else the changeset. */
export interface Ask {
  readonly range?: Range;
  readonly commit?: number;
}

/** What a manifest says that decides whether the view asked for is the one to show. */
export interface Said {
  latest: number | undefined;
  lens: Lens;
  commits: number;
}

export interface SettleIo<M> {
  read(view: View): Promise<M>;
  said(m: M): Said;
  /** A manifest of the changeset read ahead, used instead of asking once. */
  seed(): M | undefined;
  /** Called with every manifest of the changeset read, whether or not the view settles on it. */
  onChangeset(m: M): void;
}

export interface Settled<M> {
  manifest: M;
  ask: Ask;
  /** The not-found answers that gave way to another view, to be told. */
  refused: unknown[];
}

export const askOf = (c: ViewChoice): Ask => ({ range: c.range, commit: c.range ? undefined : c.commit });

export const viewOf = (ask: Ask): View =>
  ask.range || ask.commit === undefined ? rangeView(ask.range) : commitView(ask.commit);

export const showCommit = (c: ViewChoice, index: number): ViewChoice => ({
  ...c,
  range: undefined,
  commit: index,
  last: index,
});

/** A range clears the limit to the files with hunks seen against an old base; none leaves the rest as it is. */
export const showRange = (c: ViewChoice, range: Range | undefined): ViewChoice =>
  range ? { ...c, range, commit: undefined, limitedTo: undefined } : { ...c, range: undefined };

/** The changeset is reviewed in `lens` now: the commits replace a range; leaving them for a range keeps it. */
export const onLens = (c: ViewChoice, lens: Lens): ViewChoice => {
  if (lens === "perCommit") return { ...c, range: undefined, commit: c.last };
  return c.range ? c : { ...c, commit: undefined };
};

export const showChangeset = (c: ViewChoice): ViewChoice => ({ ...c, commit: undefined });

export const limit = (c: ViewChoice, files: ReadonlySet<string> | undefined): ViewChoice => ({
  ...c,
  limitedTo: files,
});

/**
 * Reads the manifest of `ask`. A range that covers every push becomes the changeset, which only the
 * manifest's latest version says, and the commit the lens asks for is shown instead of the changeset, as the
 * manifest says too. A range or commit the backend does not find gives way to the changeset. Re-asks
 * are capped, so a backend that keeps disagreeing ends on the changeset's or the last manifest.
 */
export async function settleView<M>(ask: Ask, last: number, io: SettleIo<M>): Promise<Settled<M>> {
  const refused: unknown[] = [];
  for (let attempt = 0; ; attempt++) {
    const { range, commit } = ask;
    const ofChangeset = !range && commit === undefined;
    let manifest: M;
    try {
      manifest = (ofChangeset ? io.seed() : undefined) ?? (await io.read(viewOf(ask)));
    } catch (error) {
      if (ofChangeset || !isKind(error, "notFound")) throw error;
      refused.push(error);
      ask = {};
      continue;
    }
    const said = io.said(manifest);
    if (range && normalised(range, said.latest) === undefined && attempt < 3) {
      ask = {};
      continue;
    }
    if (ofChangeset) io.onChangeset(manifest);
    if (!range) {
      const wanted = commitToShow(said.lens, said.commits, last);
      if (wanted !== commit && attempt < 3) {
        ask = { commit: wanted };
        continue;
      }
    }
    return { manifest, ask, refused };
  }
}

/** The choice with what a read settled on, unless the choice moved while it ran. */
export const commitSettled = (current: ViewChoice, from: Ask, settled: Ask): ViewChoice => {
  const now = askOf(current);
  return now.range === from.range && now.commit === from.commit
    ? { ...current, range: settled.range, commit: settled.commit }
    : current;
};
