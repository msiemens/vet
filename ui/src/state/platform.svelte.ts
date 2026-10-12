// How the platform of the pull request on view is doing, what it said of the pull request, and
// what the backend asked the viewer to hear. Syncing the threads is the backend's (SPEC I5); this
// holds the answers and the notifications.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { Notification } from "../generated/Notification";
import type { PlatformStatus } from "../generated/PlatformStatus";
import type { PlatformTraits } from "../generated/PlatformTraits";
import type { PullRequestInfo } from "../generated/PullRequestInfo";
import type { ReviewSubmitted } from "../generated/ReviewSubmitted";
import { errorText } from "../lib/errorText";
import { AccountResync } from "./accountResync";
import type { Backend } from "./backend";
import { queue } from "./queue.svelte";
import { session } from "./session.svelte";
import { threads } from "./threads.svelte";

/** How long a card of a notification that needs no answer stays in the corner. */
const CARD_MS = 12_000;

export interface Notified {
  id: number;
  notification: Notification;
  /** Unix seconds. */
  at: number;
  /** Still shown as a card in the corner. */
  card: boolean;
}

class PlatformStore {
  /** Absent for a changeset that is no pull request, and before the first sync. */
  info = $state.raw<PullRequestInfo>();
  status = $state.raw<PlatformStatus>();
  /** Newest first: what the bell lists. */
  notified = $state.raw<Notified[]>([]);
  /** The Submit review form, kept here so that switching tool windows does not empty it. */
  review = $state({
    open: false,
    verdict: undefined as string | undefined,
    body: "",
  });
  /** The toolbar's list of the outgoing comments and the vote, with the button that posts them, is open. */
  outgoingOpen = $state(false);
  /** The comment on the changeset as a whole that is being written in the Comments tool window. */
  changesetComment = $state({ composing: false, body: "" });
  /** What the platform of the last pull request on view lets the viewer do; kept while another changeset is on view. */
  traits = $state.raw<PlatformTraits>();
  /** The viewer's comments are being posted. */
  sending = $state(false);
  /** The tab the Comments tool window shows. */
  commentsTab = $state<"review" | "conversation">("review");
  /** The changeset the last `changesetOpened` put on view. */
  private opened?: ChangesetKey;
  private backend?: Backend;
  private seq = 0;
  private lastId = 0;

  start(backend: Backend): () => void {
    this.backend = backend;
    void this.reload();
    const off = backend.onEvent((event) => {
      if (event.project !== backend.project) return;
      if (event.type === "platformStatus") {
        this.status = event.status;
        void this.reload();
      } else if (event.type === "notification") {
        this.notify(event.notification);
      } else if (event.type === "changesetOpened") {
        this.opened = event.changeset;
        this.info = undefined;
        Object.assign(this.changesetComment, { composing: false, body: "" });
        void this.reload();
      } else if (event.type === "threadsChanged") {
        void this.reload();
      }
    });
    const offResync = session.onResync(() => void this.reload());
    // A credential given or removed, here or in another window, changes what the platform answers.
    const resync = new AccountResync({ sync: () => this.sync(), reload: () => this.reload() });
    const offAccounts = backend.onAccountsChanged(() => void resync.changed());
    return () => {
      off();
      offResync();
      offAccounts();
      this.backend = undefined;
      this.opened = undefined;
      this.info = undefined;
      this.status = undefined;
      this.traits = undefined;
      this.outgoingOpen = false;
    };
  }

  /** Asks again what the platform said; a changeset that is no pull request has nothing, and is not asked. */
  async reload() {
    const backend = this.backend;
    if (!backend) return;
    const seq = ++this.seq;
    const onView = this.opened ?? queue.onView;
    if (onView !== undefined && onView.type !== "pullRequest") {
      this.info = undefined;
      return;
    }
    try {
      const info = await backend.pullRequestInfo();
      if (seq === this.seq) [this.info, this.status, this.traits] = [info, info.platform, info.traits];
    } catch {
      if (seq === this.seq) this.info = undefined;
    }
  }

  /** Syncs now; a failure is told to the viewer. */
  async sync() {
    try {
      await this.backend?.syncThreads();
    } catch (error) {
      session.notice = `Could not sync comments: ${errorText(error)}`;
    }
  }

  /** Posts the drafts as one review; resolves what became of them, or nothing if it failed. */
  async submit(verdict: string, body: string): Promise<ReviewSubmitted | undefined> {
    let submitted: ReviewSubmitted | undefined;
    try {
      submitted = await this.backend?.submitReview(verdict, body.trim() || null);
    } catch (error) {
      session.notice = `Could not submit the review: ${errorText(error)}`;
      return undefined;
    }
    await threads.reload();
    return submitted;
  }

  /** Posts the drafts one after the other; resolves what became of them, or nothing if it failed. */
  async post(): Promise<ReviewSubmitted | undefined> {
    let posted: ReviewSubmitted | undefined;
    try {
      posted = await this.backend?.postDrafts();
    } catch (error) {
      session.notice = `Could not post the comments: ${errorText(error)}`;
      return undefined;
    }
    await threads.reload();
    return posted;
  }

  /** Edits one of the viewer's comments in the conversation; resolves whether the platform took it. */
  async editComment(comment: string, body: string): Promise<boolean> {
    return this.changeConversation(this.backend?.editConversationComment(comment, body), "edit the comment");
  }

  /** Deletes one of the viewer's comments in the conversation; resolves whether the platform took it. */
  async deleteComment(comment: string): Promise<boolean> {
    return this.changeConversation(this.backend?.deleteConversationComment(comment), "delete the comment");
  }

  private async changeConversation(request: Promise<void> | undefined, what: string): Promise<boolean> {
    if (!request) return false;
    try {
      await request;
    } catch (error) {
      session.notice = `Could not ${what}: ${errorText(error)}`;
      return false;
    }
    await this.reload();
    return true;
  }

  /** Stages the viewer's vote for the next post; the current vote clears the staged one. Resolves whether the backend took it. */
  async stageVote(verdict: string): Promise<boolean> {
    return this.changeVote(this.backend?.stageVote(verdict), "set the vote");
  }

  /** Drops the staged vote; resolves whether the backend did. */
  async discardVote(): Promise<boolean> {
    return this.changeVote(this.backend?.discardVote(), "discard the vote");
  }

  private async changeVote(request: Promise<void> | undefined, what: string): Promise<boolean> {
    if (!request) return false;
    try {
      await request;
    } catch (error) {
      session.notice = `Could not ${what}: ${errorText(error)}`;
      return false;
    }
    await this.reload();
    return true;
  }

  dismiss(id: number) {
    this.notified = this.notified.map((n) => (n.id === id ? { ...n, card: false } : n));
  }

  private notify(notification: Notification) {
    const id = ++this.lastId;
    this.notified = [{ id, notification, at: Math.floor(Date.now() / 1000), card: true }, ...this.notified];
    // A stopped sync stays on screen until the viewer deals with it; the rest is only told.
    if (notification.kind === "info") setTimeout(() => this.dismiss(id), CARD_MS);
  }
}

export const platform = new PlatformStore();
