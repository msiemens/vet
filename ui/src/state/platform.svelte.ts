// How the platform of the pull request on view is doing, what it said of the pull request, and
// what the backend asked the viewer to hear. Syncing the threads is the backend's (SPEC I5); this
// holds the answers and the notifications.

import type { ChangesetKey } from "../generated/ChangesetKey";
import type { Notification } from "../generated/Notification";
import type { PlatformStatus } from "../generated/PlatformStatus";
import type { PlatformTraits } from "../generated/PlatformTraits";
import type { PullRequestInfo } from "../generated/PullRequestInfo";
import type { ReviewSubmitted } from "../generated/ReviewSubmitted";
import type { Verdict } from "../generated/Verdict";
import { HoldTimer } from "../lib/ui/holdTimer";
import { AccountResync } from "./accountResync";
import type { Backend } from "./backend";
import { followProject, whileOpen } from "./followProject";
import { notices } from "./notices.svelte";
import { queue } from "./queue.svelte";
import { Resource } from "./resource";
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
    verdict: undefined as Verdict | undefined,
    body: "",
  });
  /** The toolbar's list of the outgoing comments and the vote, with the button that posts them, is open. */
  outgoingOpen = $state(false);
  /** While that popover is open it lists the votes to pick from instead. */
  votePicking = $state(false);
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
  private reads = new Resource<PullRequestInfo>({
    apply: (info) => ([this.info, this.status, this.traits] = [info, info.platform, info.traits]),
    failed: () => (this.info = undefined),
  });
  private lastId = 0;
  private cardTimers = new Map<number, HoldTimer>();

  constructor() {
    whileOpen((backend) => this.start(backend));
  }

  private start(backend: Backend): () => void {
    this.backend = backend;
    const stop = followProject(backend, {
      reloadOn: ["platformStatus", "threadsChanged"],
      reload: () => void this.reload(),
      cancel: () => this.reads.cancel(),
      clear: () => (this.info = undefined),
      event: (event) => {
        if (event.type === "platformStatus") this.status = event.status;
        else if (event.type === "notification") this.notify(event.notification);
      },
      opening: (opening) => {
        if (opening.type !== "opened") return;
        this.opened = opening.changeset;
        Object.assign(this.changesetComment, { composing: false, body: "" });
      },
    });
    // A credential given or removed, here or in another window, changes what the platform answers.
    const resync = new AccountResync({ sync: () => this.sync(), reload: () => this.reload() });
    const offAccounts = backend.onAccountsChanged(() => void resync.changed());
    return () => {
      stop();
      offAccounts();
      this.reads.cancel();
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
    const onView = this.opened ?? queue.onView;
    if (onView !== undefined && onView.type !== "pullRequest") {
      this.reads.cancel();
      this.info = undefined;
      return;
    }
    await this.reads.load(() => backend.pullRequestInfo());
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
    await threads.reload();
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

  /** What the viewer has not read of the pull request as a whole. */
  get unreadCount(): number {
    return this.info?.unread.all ?? 0;
  }

  /** The viewer's comments and staged vote that wait to be posted. */
  get outgoingCount(): number {
    return this.info?.outgoing.count ?? 0;
  }

  /** Shows the outgoing comments and the vote, with the button that posts them; `picking` lists the votes to pick from instead. */
  openOutgoing(picking: boolean) {
    this.votePicking = picking;
    this.outgoingOpen = true;
  }

  /** Shows the Comments tool window's Conversation tab. */
  showConversation() {
    this.commentsTab = "conversation";
  }

  /** Opens the form for a comment on the changeset as a whole. */
  composeChangesetComment() {
    this.changesetComment.composing = true;
  }

  private notify(notification: Notification) {
    const id = ++this.lastId;
    this.notified = [{ id, notification, at: Math.floor(Date.now() / 1000), card: true }, ...this.notified];
    // A stopped sync stays on screen until the viewer deals with it; the rest is only told.
    if (notification.kind === "info") this.cardTimers.set(id, new HoldTimer(() => this.dismiss(id), CARD_MS));
  }
}

export const platform = new PlatformStore();
