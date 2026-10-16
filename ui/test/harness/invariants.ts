// What the stores hold, compared with a fresh read of the backend once a step has settled. Each
// difference is a violation of an invariant, named so that `known.ts` can list the ones that are
// known bugs. What the editor views draw is compared in `browser.ts`.

import type { Base } from "../../src/generated/Base.ts";
import type { Manifest } from "../../src/generated/Manifest.ts";
import type { RowReview } from "../../src/generated/RowReview.ts";
import type { Thread } from "../../src/generated/Thread.ts";
import type { View } from "../../src/generated/View.ts";
import type { Backend } from "../../src/state/backend.ts";
import { keyId } from "../../src/state/changesetKey.ts";
import { opened } from "../../src/state/opened.svelte.ts";
import { changesOf, reviewedOf } from "../../src/state/queueEntries.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { reviewShown } from "../../src/state/reviewShown.svelte.ts";
import { versions } from "../../src/state/versions.svelte.ts";
import { CHANGESET } from "../../src/state/view.ts";

export interface Violation {
  invariant: string;
  /** What the window showed: the changeset, an interdiff or a commit. */
  view: View["type"];
  what: string;
  ui: unknown;
  backend: unknown;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** What of a base identifies it to the viewer. */
const baseOf = (base: Base) => ({
  commit: base.commit.id,
  containedIn: base.containedIn,
  into: base.into,
  headOffBase: base.headOffBase,
});

/** The part of a row's review that the queue shows without being asked again. */
const rowReview = (review: RowReview | undefined) =>
  review && {
    state: review.state,
    version: "version" in review ? review.version : undefined,
    lastReviewed: "lastReviewed" in review ? review.lastReviewed : undefined,
  };

export async function checkInvariants(other: Backend): Promise<Violation[]> {
  const current = opened.session;
  const layout = current?.layout;
  const facts = current?.facts;
  if (!current || !layout || !facts || !current.viewCompletion)
    return [{ invariant: "manifest", view: "changeset", what: "no manifest", ui: null, backend: null }];
  const view = current.view;
  const found: Violation[] = [];
  const differ = (invariant: string, what: string, ui: unknown, backend: unknown) => {
    if (!same(ui, backend)) found.push({ invariant, view: view.type, what, ui, backend });
  };

  const [fresh, whole, listed, backendVersions, commits, backendWhole, backendRange] = await Promise.all([
    other.manifest(view),
    other.manifest(CHANGESET),
    other.listChangesets(),
    other.versions(),
    other.commits().then((r) => r.commits),
    other.threads(CHANGESET),
    current.range ? other.threads(view) : undefined,
  ]);

  differ("shownView", "the view the layout is of", layout.shown.view, view);
  differ(
    "sessionKey",
    "the changeset of the session on view",
    opened.session && keyId(opened.session.key),
    keyId(facts.target.changeset),
  );

  differ(
    "fileReview",
    "the paths of the view",
    current.paths,
    fresh.files.map((f) => f.path),
  );
  for (const file of fresh.files) {
    differ("fileReview", `state of ${file.path}`, current.state(file.path), {
      review: file.review,
      completion: file.completion,
    });
  }
  differ("viewCompletion", "completion of the view", current.viewCompletion, fresh.completion);
  differ("changesetCompletion", "completion of the changeset", facts.completion, whole.changesetCompletion);
  differ("base", "base of the view", baseOf(layout.base), baseOf(fresh.base));
  differ("reviewInfo", "review of the changeset", versionOf(facts), versionOf(fresh));

  // The queue panel and Problems read the store's whole-changeset view; the selected row's own
  // numbers come from it, whatever view the review shows.
  const wholeFrame = current.wholeFrame;
  differ(
    "wholeFrame",
    "the whole-changeset view",
    wholeFrame && {
      completion: wholeFrame.facts.completion,
      delta: wholeFrame.layout.delta,
      files: wholeFrame.layout.paths,
      base: baseOf(wholeFrame.layout.base),
    },
    {
      completion: whole.completion,
      delta: whole.delta,
      files: whole.files.map((f) => f.path),
      base: baseOf(whole.base),
    },
  );
  const entry = queue.entries.find((e) => e.onView);
  differ(
    "onViewRowWhole",
    "the selected row's numbers from the whole view",
    entry && { changes: entry.changes, reviewed: entry.reviewed },
    entry && {
      changes: changesOf(entry, wholeFrame?.layout.delta, queue.lines.get(keyId(entry.key))),
      reviewed: reviewedOf(entry.review, wholeFrame?.facts.completion),
    },
  );

  const key = keyId(facts.target.changeset);
  const row = (rows: { key: Parameters<typeof keyId>[0]; review: RowReview }[]) =>
    rowReview(rows.find((r) => keyId(r.key) === key)?.review);
  differ("queueRow", "review of the on-view row", row(queue.local), row(listed.rows));

  // The versions and the per-commit list are the review screen's: while it is away its stores hold
  // nothing, and the queue asks the backend for neither.
  differ(
    "versions",
    "the versions",
    reviewShown.on ? versions.items.map((v) => [v.n, v.reviewedHere]) : versions.items,
    reviewShown.on ? backendVersions.map((v) => [v.n, v.reviewedHere]) : [],
  );
  const unit = (list: typeof commits) => list.map((c) => [c.commit.id, c.completion]);
  const wantedCommits = reviewShown.on && facts.lens === "perCommit";
  differ(
    "commits",
    "progress of the commits",
    wantedCommits ? unit(current.commits) : current.commits,
    wantedCommits ? unit(commits) : [],
  );
  // The queue panel lists the whole changeset's threads, whatever range the review shows; the range's own
  // are held only while the review screen shows that range.
  const threadsOf = (list: readonly Thread[]) => list.map((t) => [t.id, t.status, t.messages.length]);
  const threads = current.threads;
  if (threads.whole?.revision === backendWhole.revision) {
    differ(
      "wholeThreads",
      "the threads the queue panel lists",
      threadsOf(threads.wholeItems),
      threadsOf(backendWhole.threads),
    );
  } else
    differ("wholeThreads", "the revision the threads were read at", threads.whole?.revision, backendWhole.revision);
  if (backendRange && reviewShown.on) {
    const held = threads.of(current.range);
    if (held?.revision === backendRange.revision) {
      differ("rangeThreads", "the threads of the range", threadsOf(held.items), threadsOf(backendRange.threads));
    } else
      differ("rangeThreads", "the revision the range's threads were read at", held?.revision, backendRange.revision);
  } else differ("rangeThreads", "the threads of a range that is not on view", threads.range?.key, undefined);
  return found;
}

const versionOf = (m: { review: Manifest["review"] }) => ({
  state: m.review.state,
  version: "version" in m.review ? m.review.version : undefined,
});
