// The requests that mark, flag, acknowledge or comment, as one view of the changeset sends them: the
// changeset store for the view on screen, a `ViewScope` for the view a tab stays at. A request carries the
// `Shown` of the view that sent it; the host decides whether that is still the view it holds, takes the
// answer into its review state, and says what a failed request means.

import type { BaseDiff } from "../generated/BaseDiff";
import type { CommentAt } from "../generated/CommentAt";
import type { HunkState } from "../generated/HunkState";
import type { NonTextKind } from "../generated/NonTextKind";
import { STALE_NOTICE } from "../lib/notice";
import { RequestError } from "../lib/requestError";
import type { Backend } from "./backend";
import { notices } from "./notices.svelte";
import { atRevision } from "./revision";
import { type Shown, isCurrent } from "./shown";

/** What a request that marks answers with: a file's review state. */
export type Answer = Awaited<ReturnType<Backend["toggleHunk"]>>;

/** What the requests ask of the view that sends them. */
export interface MarkHost {
  backend(): Backend;
  /** Whether a request built for `shown` may go out; if not, the host says why. */
  admit(shown: Shown): boolean;
  /** What the host's review state is of now. */
  current(): Shown | undefined;
  /** Takes an answer into the review state. */
  apply(update: Answer, shown: Shown): void;
  /** Reads the host's state anew, after a request was rejected. */
  refresh(): Promise<unknown>;
  /** The states of a file's hunks as the host holds them. */
  hunks(path: string): readonly HunkState[];
  /** Whether the host is gone (its changeset was switched away from): a failure of its requests is not told. */
  ended?(): boolean;
}

/**
 * Tells the viewer why a request failed. When the backend says the revision is outdated, or the
 * manifest has moved on from the revision the request was sent for before the backend could say so,
 * the views are about to be replaced and the action has to be repeated.
 */
export function reportFailure(error: unknown, sent: number | undefined, current: number | undefined) {
  if (sent !== undefined && current !== undefined && current !== sent) notices.show(STALE_NOTICE);
  else notices.failed("Request failed", error);
}

export function reviewMarks(host: MarkHost) {
  const fail = (error: unknown, sent: number) => {
    if (!host.ended?.()) reportFailure(error, sent, host.current()?.revision);
  };

  /**
   * Applies the answers of a request unless the view it was sent for has been replaced (the manifest of the
   * new one has the state). A rejected request is normally a stale revision: the state is read anew and the
   * viewer told to repeat the action.
   */
  async function review(sent: Shown, request: Promise<Answer | Answer[]>) {
    try {
      const updates = [await request].flat();
      if (isCurrent(sent, host.current())) for (const update of updates) host.apply(update, sent);
    } catch (error) {
      await host.refresh().catch(() => undefined);
      fail(error, sent.revision);
    }
  }

  async function toggleHunk(shown: Shown, path: string, hunk: number) {
    if (!host.admit(shown)) return;
    await review(shown, host.backend().toggleHunk(shown.revision, path, hunk, shown.view));
  }

  return {
    review,
    toggleHunk,
    async toggleFlag(shown: Shown, path: string, hunk: number) {
      if (!host.admit(shown)) return;
      await review(shown, host.backend().toggleFlag(shown.revision, path, hunk, shown.view));
    },
    async toggleItem(shown: Shown, path: string, kind: NonTextKind) {
      if (!host.admit(shown)) return;
      await review(shown, host.backend().toggleItem(shown.revision, path, kind, shown.view));
    },
    async toggleFileSeen(shown: Shown, path: string) {
      if (!host.admit(shown)) return;
      await review(shown, host.backend().toggleFileSeen(shown.revision, path, shown.view));
    },
    async setFilesSeen(shown: Shown, paths: string[], seen: boolean) {
      if (!host.admit(shown)) return;
      await review(shown, host.backend().setFilesSeen(shown.revision, paths, seen, shown.view));
    },
    /**
     * Marks the hunks of a file seen against an old base seen again. A range of versions or a commit has no
     * such request: its hunks are marked one by one, as the backend flips them.
     */
    async reconfirmFile(shown: Shown, path: string) {
      if (!host.admit(shown)) return;
      if (shown.view.type === "changeset") {
        await review(shown, host.backend().reconfirmFile(shown.revision, path));
        return;
      }
      for (const [hunk, state] of host.hunks(path).entries()) {
        if (state === "seenAgainstOldBase") await toggleHunk(shown, path, hunk);
      }
    },
    /** The base diff of a hunk seen against an old base; `undefined` after showing why it could not be had. */
    async baseDiff(shown: Shown, path: string, hunk: number): Promise<BaseDiff | undefined> {
      if (!host.admit(shown)) return undefined;
      try {
        return atRevision(host.current()?.revision, await host.backend().baseDiff(shown.revision, path, hunk));
      } catch (error) {
        fail(error, shown.revision);
        return undefined;
      }
    },
    /** Where a comment on a part of the file attaches in `shown`; rejects if `shown` is not current. */
    async commentTarget(shown: Shown, path: string, at: CommentAt) {
      if (!host.admit(shown)) throw new RequestError("outdated", STALE_NOTICE);
      const reply = atRevision(
        host.current()?.revision,
        await host.backend().commentTarget(shown.revision, path, at, shown.view),
      );
      if (!reply) throw new RequestError("outdated", STALE_NOTICE);
      return reply;
    },
  };
}
