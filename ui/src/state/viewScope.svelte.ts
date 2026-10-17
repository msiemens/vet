// A view of the changeset that file tabs read while it is not the one the review screen shows: the
// changeset as a whole, or one of its commits. A tab opened at a commit stays at it, and a tab opened on
// the whole changeset stays on that when a commit is selected, so each reads its own manifest and holds
// its own review state, which the store of the view on screen cannot give it.

import type { BaseDiff } from "../generated/BaseDiff";
import type { CommentAt } from "../generated/CommentAt";
import type { CommitUnit } from "../generated/CommitUnit";
import type { NonTextKind } from "../generated/NonTextKind";
import type { Backend } from "./backend";
import type { CommitList } from "./commitList";
import { HeldView } from "./heldView.svelte";
import { notices } from "./notices.svelte";
import { Resource } from "./resource";
import type { Answer } from "./reviewMarks";
import { type FileLayout, type Parts, type ViewLayout, partsOf } from "./reviewModel";
import type { ReviewSource } from "./reviewSource";
import type { Shown } from "./shown";
import { commitView } from "./view";

/** What a file tab reads and does in its view: the changeset store implements it for the view on screen, `ViewScope` for the others. */
export interface ViewSource extends ReviewSource {
  /** The view is gone from the changeset (a rewritten commit); only a `ViewScope` of a commit can be. */
  readonly gone?: boolean;
  /** What the layout is of; none before the first manifest. Reads nothing reactively. */
  currentShown(): Shown | undefined;
  currentLayout(): ViewLayout | undefined;
  /** The structure of a file of the view. */
  layoutOf(path: string): FileLayout | undefined;
  /** Whether a request built for `shown` still is about this view; if not, the viewer is told so. */
  admit(shown: Shown): boolean;
  toggleHunk(shown: Shown, path: string, hunk: number): Promise<void>;
  toggleFlag(shown: Shown, path: string, hunk: number): Promise<void>;
  toggleItem(shown: Shown, path: string, kind: NonTextKind): Promise<void>;
  toggleFileSeen(shown: Shown, path: string): Promise<void>;
  reconfirmFile(shown: Shown, path: string): Promise<void>;
  baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined>;
  commentTarget(shown: Shown, path: string, at: CommentAt): ReturnType<Backend["commentTarget"]>;
}

import type { ScopeSpec } from "./scopeSpec";
export { scopeKey } from "./scopeSpec";
export type { ScopeSpec };

interface Loaded {
  parts: Parts;
  commit?: CommitUnit;
}

/** A commit of the changeset, by full id: the view a tab reads that is not the one on screen. */
export type CommitSpec = Extract<ScopeSpec, { type: "commit" }>;

class Scope extends HeldView implements ViewSource {
  /** The commit of a commit's scope, once the commits are read. */
  commit = $state.raw<CommitUnit>();
  /** The commit is not among the changeset's any more. */
  gone = $state(false);
  /** A failure of reading was reported and nothing has been read since. */
  private reported = false;
  private pending: Promise<void> = Promise.resolve();
  private loads = new Resource<Loaded | "gone" | "stale">({ apply: (loaded) => this.apply(loaded) });

  /** `marked` is told of every answer a mark got here, since the view on screen may show the same lines. */
  constructor(
    private api: Backend,
    readonly spec: CommitSpec,
    private marked: (shown: Shown, update: Answer) => void,
    private commits: CommitList,
    private revision: () => number | undefined,
  ) {
    super();
  }

  protected override client(): Backend {
    return this.api;
  }

  protected override refresh(): Promise<unknown> {
    return this.load();
  }

  protected override applyAnswer(update: Answer, shown: Shown) {
    this.putAnswer(update);
    this.marked(shown, update);
  }

  /** Reads the manifest of the view; the latest read counts. */
  load(): Promise<void> {
    this.pending = this.loads
      .load(() => this.fetch())
      .then(
        () => {
          this.reported = false;
        },
        (error: unknown) => {
          if (!this.reported) notices.report(error);
          this.reported = true;
        },
      );
    return this.pending;
  }

  /** Settles when the read underway, if there is one, is over. */
  whenLoaded(): Promise<void> {
    return this.pending;
  }

  /**
   * The backend announced a mark of the changeset's file, as the answer to a request and as an event.
   * A commit has no word of its lines in it, so it reads again, unless it has what the update says
   * already (its own mark).
   */
  heard(update: Answer, revision: number) {
    if (revision !== this.layout?.shown.revision || update.seq <= this.seen) return;
    void this.load();
  }

  /**
   * The commit's manifest and the commits are paired by revision. A view the changeset has moved
   * past reads nothing: `"stale"` keeps what the scope holds and says nothing, since the store's
   * next reload reads the newer revision; only a real failure is reported.
   */
  private async fetch(): Promise<Loaded | "gone" | "stale"> {
    const id = this.spec.id;
    const revision = this.revision();
    if (revision === undefined) return "stale";
    const commits = await this.commits.at(revision);
    if (!commits) return "stale";
    const index = commits.findIndex((c) => c.commit.id === id);
    if (index < 0) return "gone";
    const parts = partsOf(await this.api.manifest(commitView(index)), index);
    if (parts.revision !== revision) return "stale";
    return { parts, commit: commits[index] };
  }

  private apply(loaded: Loaded | "gone" | "stale") {
    if (loaded === "gone") {
      this.gone = true;
      this.cells.clearedAll();
      return;
    }
    if (loaded === "stale") return;
    const { parts, commit } = loaded;
    this.gone = false;
    this.commit = commit;
    this.take(parts);
  }
}

export type ViewScope = Scope;

/** A scope of the commit `spec`; `marked` is told of every answer a mark got in it. */
export const newScope = (
  backend: Backend,
  spec: CommitSpec,
  marked: (shown: Shown, update: Answer) => void,
  commits: CommitList,
  revision: () => number | undefined,
): ViewScope => new Scope(backend, spec, marked, commits, revision);
