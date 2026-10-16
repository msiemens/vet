// What the header says of the review, derived from the stores that own the data: the scope chip
// (what the view shows) and the base note (what the changes are measured from). The wording itself
// is in `scopeText`.

import { minute } from "../../state/minute.svelte";
import { opened } from "../../state/opened.svelte";
import { pendingEdits } from "../../state/pendingEdits.svelte";
import { type BaseNote, type ScopeChip, baseNote, scopeChip } from "./scopeText";

class HeaderFacts {
  /** The chip for what the review view shows; none before the first manifest. */
  readonly scope = $derived.by((): ScopeChip | undefined => {
    const session = opened.session;
    const facts = session?.facts;
    if (!session || !facts) return undefined;
    const commit =
      session.shownCommit === undefined
        ? undefined
        : {
            position: session.shownCommit + 1,
            of: facts.commits,
            shortId: session.shownCommitUnit?.commit.shortId,
            subject: session.shownCommitUnit?.commit.subject,
          };
    return scopeChip({
      range: session.range,
      latest: session.versions.items.at(-1)?.n,
      commit,
      live: facts.target.changeset.type === "workingTree",
      pending: pendingEdits.waiting,
    });
  });

  /** The base note, where the header should say what the changes are measured from; none usually. */
  readonly note = $derived.by((): BaseNote | undefined => {
    const facts = opened.session?.facts;
    return facts && baseNote(facts.base, facts.target.changeset.type, minute.now);
  });
}

export const headerFacts = new HeaderFacts();
