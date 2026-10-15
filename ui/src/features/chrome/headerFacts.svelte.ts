// What the header says of the review, derived from the stores that own the data: the scope chip
// (what the view shows) and the base note (what the changes are measured from). The wording itself
// is in `scopeText`.

import { changeset } from "../../state/changeset.svelte";
import { minute } from "../../state/minute.svelte";
import { pendingEdits } from "../../state/pendingEdits.svelte";
import { versions } from "../../state/versions.svelte";
import { type BaseNote, type ScopeChip, baseNote, scopeChip } from "./scopeText";

class HeaderFacts {
  /** The chip for what the review view shows; none before the first manifest. */
  readonly scope = $derived.by((): ScopeChip | undefined => {
    const facts = changeset.facts;
    if (!facts) return undefined;
    const commit =
      changeset.shownCommit === undefined
        ? undefined
        : {
            position: changeset.shownCommit + 1,
            of: facts.commits,
            shortId: changeset.shownCommitUnit?.commit.shortId,
            subject: changeset.shownCommitUnit?.commit.subject,
          };
    return scopeChip({
      range: changeset.range,
      latest: versions.items.at(-1)?.n,
      commit,
      live: facts.target.changeset.type === "workingTree",
      pending: pendingEdits.waiting,
    });
  });

  /** The base note, where the header should say what the changes are measured from; none usually. */
  readonly note = $derived.by((): BaseNote | undefined => {
    const facts = changeset.facts;
    return facts && baseNote(facts.base, facts.target.changeset.type, minute.now);
  });
}

export const headerFacts = new HeaderFacts();
