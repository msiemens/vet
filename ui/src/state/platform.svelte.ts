// How the platform is doing and what the backend asked the viewer to hear: the window's, whatever
// changeset is on view. What it said of the pull request on view is the session's
// (`pullRequest.svelte.ts`). Syncing the threads is the backend's; this holds the status
// and the notifications, and none of the Comments tool window's or the outgoing popover's own flags
// (`features/comments/commentsUi.svelte.ts`).

import { untrack } from "svelte";
import type { Notification } from "../generated/Notification";
import type { PlatformStatus } from "../generated/PlatformStatus";
import type { ReviewSubmitted } from "../generated/ReviewSubmitted";
import type { Verdict } from "../generated/Verdict";
import { HoldTimer } from "../lib/ui/holdTimer";
import { AccountResync } from "./accountResync";
import type { Backend } from "./backend";
import { whileOpen } from "./whileOpen";
import { notices } from "./notices.svelte";
import { opened } from "./opened.svelte";

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
  /** The last the platform said of itself, in an answer about a pull request or pushed; before either, absent. */
  status = $state.raw<PlatformStatus>();
  /** Newest first: what the bell lists. */
  notified = $state.raw<Notified[]>([]);
  private backend?: Backend;
  private lastId = 0;
  private cardTimers = new Map<number, HoldTimer>();

  constructor() {
    whileOpen((backend) => this.start(backend));
    // What the platform says of the pull request on view is also what it says of itself.
    $effect.root(() => {
      $effect(() => {
        const said = opened.session?.pullRequest?.info?.platform;
        if (said) untrack(() => (this.status = said));
      });
    });
  }

  private start(backend: Backend): () => void {
    this.backend = backend;
    const stop = backend.onEvent((event) => {
      if (event.project !== backend.project) return;
      if (event.type === "platformStatus") this.status = event.status;
      else if (event.type === "notification") this.notify(event.notification);
    });
    // A credential given or removed, here or in another window, changes what the platform answers.
    const resync = new AccountResync({ sync: () => this.sync(), reload: () => this.reload() });
    const offAccounts = backend.onAccountsChanged(() => void resync.changed());
    return () => {
      stop();
      offAccounts();
      this.backend = undefined;
      this.status = undefined;
    };
  }

  private async reload() {
    await opened.session?.pullRequest?.reload();
  }

  /** Syncs now; a failure is told to the viewer. */
  async sync() {
    try {
      await this.backend?.syncThreads();
    } catch (error) {
      notices.failed("Could not sync comments", error);
    }
  }

  /** Posts the drafts as one review; resolves what became of them, or nothing if it failed. */
  async submit(verdict: Verdict, body: string): Promise<ReviewSubmitted | undefined> {
    let submitted: ReviewSubmitted | undefined;
    try {
      submitted = await this.backend?.submitReview(verdict, body.trim() || null);
    } catch (error) {
      notices.failed("Could not submit the review", error);
      return undefined;
    }
    await opened.session?.threads.reload();
    return submitted;
  }

  /** Posts the drafts one after the other; resolves what became of them, or nothing if it failed. */
  async post(): Promise<ReviewSubmitted | undefined> {
    let posted: ReviewSubmitted | undefined;
    try {
      posted = await this.backend?.postDrafts();
    } catch (error) {
      notices.failed("Could not post the comments", error);
      return undefined;
    }
    await opened.session?.threads.reload();
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
      notices.failed(`Could not ${what}`, error);
      return false;
    }
    await this.reload();
    return true;
  }

  /** Stages the viewer's vote for the next post; the current vote clears the staged one. Resolves whether the backend took it. */
  async stageVote(verdict: Verdict): Promise<boolean> {
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
      notices.failed(`Could not ${what}`, error);
      return false;
    }
    await this.reload();
    return true;
  }

  /**
   * Marks the pull request on view's comments read; the backend moves the time the viewer last
   * looked and pushes the new counts when they change. Called while the Comments tool is open.
   */
  async markCommentsSeen(): Promise<void> {
    try {
      await this.backend?.markCommentsSeen();
    } catch (error) {
      notices.report(error);
    }
  }

  dismiss(id: number) {
    this.cardTimers.get(id)?.stop();
    this.cardTimers.delete(id);
    this.notified = this.notified.map((n) => (n.id === id ? { ...n, card: false } : n));
  }

  /** The viewer is reading or selecting the card: it stays until `releaseCard`. */
  holdCard(id: number) {
    this.cardTimers.get(id)?.hold();
  }

  releaseCard(id: number) {
    this.cardTimers.get(id)?.release();
  }

  /** The platform says the token is missing or no longer works, so nothing can be asked of it. */
  get signedOut(): boolean {
    const connection = this.status?.connection.type;
    return connection === "signedOut" || connection === "tokenExpired";
  }

  private notify(notification: Notification) {
    const id = ++this.lastId;
    this.notified = [{ id, notification, at: Math.floor(Date.now() / 1000), card: true }, ...this.notified];
    // A stopped sync stays on screen until the viewer deals with it; the rest is only told.
    if (notification.type !== "syncStopped") {
      this.cardTimers.set(id, new HoldTimer(() => this.dismiss(id), CARD_MS));
    }
  }
}

export const platform = new PlatformStore();
