// What the stores hold, compared with a fresh read of the backend once a step has settled. Each
// difference is a violation of an invariant, named so that `known.ts` can list the ones that are
// known bugs.

import type { Base } from "../../src/generated/Base.ts";
import type { FileChange } from "../../src/generated/FileChange.ts";
import type { Manifest } from "../../src/generated/Manifest.ts";
import type { RowReview } from "../../src/generated/RowReview.ts";
import type { View } from "../../src/generated/View.ts";
import type { Backend } from "../../src/state/backend.ts";
import { keyId } from "../../src/state/changesetKey.ts";
import { changeset } from "../../src/state/changeset.svelte.ts";
import { queue } from "../../src/state/queue.svelte.ts";
import { rangeKey } from "../../src/state/timelineRange.ts";
import { threads } from "../../src/state/threads.svelte.ts";
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

type FileState = Pick<FileChange, "review" | "completion">;

/**
 * What an editor view holds of the review state, as the documented channels keep it: built from the
 * files' current state when the revision, the range or commit or the file filter changes, and patched
 * by what they read of the files written, as `ReviewController` does. The editor views are not
 * Svelte-free of this contract, so the harness models them here, polled between the steps' requests.
 */
export class ViewMirror {
  readonly files = new Map<string, FileState>();
  private built?: string;

  constructor() {
    changeset.subscribe((paths) => {
      for (const path of this.files.keys()) {
        const file = paths === "all" || paths.has(path) ? changeset.file(path) : undefined;
        if (file) this.files.set(path, { review: file.review, completion: file.completion });
      }
    });
  }

  /** Rebuilds the views if what they are built for changed. */
  follow() {
    const manifest = changeset.manifest;
    if (!manifest) return;
    const key = JSON.stringify([manifest.revision, changeset.viewKey, [...(changeset.baseChanged ?? [])]]);
    if (key === this.built) return;
    this.built = key;
    this.files.clear();
    for (const file of changeset.currentFiles())
      this.files.set(file.path, { review: file.review, completion: file.completion });
  }
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** What of a base identifies it to the viewer. */
const baseOf = (base: Base) => ({
  name: base.name,
  commit: base.commit.id,
  behind: base.behind,
  containedIn: base.containedIn,
});

/** The part of a row's review that the queue shows without being asked again. */
const rowReview = (review: RowReview | undefined) =>
  review && {
    state: review.state,
    version: "version" in review ? review.version : undefined,
    lastReviewed: "lastReviewed" in review ? review.lastReviewed : undefined,
  };

export async function checkInvariants(other: Backend, mirror: ViewMirror): Promise<Violation[]> {
  const manifest: Manifest | undefined = changeset.manifest;
  if (!manifest) return [{ invariant: "manifest", view: "changeset", what: "no manifest", ui: null, backend: null }];
  const view = changeset.view;
  const found: Violation[] = [];
  const differ = (invariant: string, what: string, ui: unknown, backend: unknown) => {
    if (!same(ui, backend)) found.push({ invariant, view: view.type, what, ui, backend });
  };

  const [fresh, whole, listed, backendVersions, commits, backendThreads] = await Promise.all([
    other.manifest(view),
    other.manifest(CHANGESET),
    other.listChangesets(),
    other.versions(),
    other.commits(),
    other.threads(changeset.range ? view : CHANGESET),
  ]);

  const shown = manifest.interdiff
    ? { type: "interdiff", from: manifest.interdiff.from, to: manifest.interdiff.to }
    : changeset.shownCommit === undefined
      ? { type: "changeset" }
      : { type: "commit", index: changeset.shownCommit };
  differ("shownView", "the view the manifest is of", shown, view);

  differ(
    "fileReview",
    "the paths of the view",
    changeset.paths,
    fresh.files.map((f) => f.path),
  );
  for (const file of fresh.files) {
    const held = changeset.file(file.path);
    differ("fileReview", `state of ${file.path}`, held && { review: held.review, completion: held.completion }, {
      review: file.review,
      completion: file.completion,
    });
  }
  differ("viewCompletion", "completion of the view", manifest.completion, fresh.completion);
  differ("changesetCompletion", "completion of the changeset", manifest.changesetCompletion, whole.changesetCompletion);
  differ("base", "base of the view", baseOf(manifest.base), baseOf(fresh.base));
  differ("reviewInfo", "review of the changeset", versionOf(manifest), versionOf(fresh));

  // The queue and the switcher show the changeset's own manifest as the on-view row's numbers, whatever view the review shows.
  const own = (m: Manifest | undefined) =>
    m && {
      completion: m.completion,
      delta: m.delta,
      files: m.files.map((f) => f.path),
      base: baseOf(m.base),
    };
  differ(
    "onViewRowFacts",
    "what the on-view row of the queue takes from the manifest",
    own(queue.onViewManifest),
    own(whole),
  );

  const key = keyId(manifest.target.changeset);
  const row = (rows: { key: Parameters<typeof keyId>[0]; review: RowReview }[]) =>
    rowReview(rows.find((r) => keyId(r.key) === key)?.review);
  differ("queueRow", "review of the on-view row", row(queue.local), row(listed.rows));

  differ(
    "versions",
    "the versions",
    versions.items.map((v) => [v.n, v.reviewedHere]),
    backendVersions.map((v) => [v.n, v.reviewedHere]),
  );
  const unit = (list: typeof commits) => list.map((c) => [c.commit.id, c.completion]);
  differ(
    "commits",
    "progress of the commits",
    manifest.lens === "perCommit" ? unit(changeset.commits) : [],
    manifest.lens === "perCommit" ? unit(commits) : [],
  );
  if (threads.revision === backendThreads.revision && threads.key === rangeKey(changeset.range)) {
    differ(
      "threads",
      "the threads",
      threads.items.map((t) => [t.id, t.status, t.messages.length]),
      backendThreads.threads.map((t) => [t.id, t.status, t.messages.length]),
    );
  } else differ("threads", "the revision the threads were read at", threads.revision, backendThreads.revision);

  for (const file of fresh.files) {
    differ("viewMirror", `state an editor view holds for ${file.path}`, mirror.files.get(file.path), {
      review: file.review,
      completion: file.completion,
    });
  }
  return found;
}

const versionOf = (m: Manifest) => ({
  state: m.review.state,
  version: "version" in m.review ? m.review.version : undefined,
});
