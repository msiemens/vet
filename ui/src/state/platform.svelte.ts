// How the platform of the pull request on view is doing, what it said of the pull request, and
// what the backend asked the viewer to hear. Syncing the threads is the backend's (SPEC I5); this
// holds the answers and the notifications.

import type { Notification } from "../generated/Notification";
import type { PlatformStatus } from "../generated/PlatformStatus";
import type { PullRequestInfo } from "../generated/PullRequestInfo";
import type { ReviewSubmitted } from "../generated/ReviewSubmitted";
import { errorText } from "../lib/errorText";
import type { Backend } from "./backend";
import { session } from "./session.svelte";

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
  /** Counts the asks for Settings on the Accounts page; the app window opens the dialog on each. */
  accountsRequests = $state(0);
  /** The Submit review form, kept here so that switching tool windows does not empty it. */
  review = $state({ open: false, verdict: undefined as string | undefined, body: "" });
  private signingIn = false;
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
        this.info = undefined;
        void this.reload();
      } else if (event.type === "threadsChanged") {
        void this.reload();
      }
    });
    return () => {
      off();
      this.backend = undefined;
      this.info = undefined;
      this.status = undefined;
    };
  }

  /** Asks again what the platform said; a changeset that is no pull request has nothing. */
  async reload() {
    const backend = this.backend;
    if (!backend) return;
    const seq = ++this.seq;
    try {
      const info = await backend.pullRequestInfo();
      if (seq === this.seq) [this.info, this.status] = [info, info.platform];
    } catch {
      if (seq === this.seq) this.info = undefined;
    }
  }

  /** Syncs now, for after signing in again. */
  async sync() {
    try {
      await this.backend?.syncThreads();
    } catch (error) {
      session.notice = `Could not sync comments: ${errorText(error)}`;
    }
  }

  /** Posts the drafts as one review; resolves what became of them, or nothing if it failed. */
  async submit(verdict: string, body: string): Promise<ReviewSubmitted | undefined> {
    try {
      return await this.backend?.submitReview(verdict, body.trim() || null);
    } catch (error) {
      session.notice = `Could not submit the review: ${errorText(error)}`;
      return undefined;
    }
  }

  /** Posts the drafts one after the other; resolves what became of them, or nothing if it failed. */
  async post(): Promise<ReviewSubmitted | undefined> {
    try {
      return await this.backend?.postDrafts();
    } catch (error) {
      session.notice = `Could not post the comments: ${errorText(error)}`;
      return undefined;
    }
  }

  /** Sets the viewer's vote; resolves whether the platform took it. */
  async vote(verdict: string): Promise<boolean> {
    try {
      await this.backend?.setVote(verdict);
      return true;
    } catch (error) {
      session.notice = `Could not set the vote: ${errorText(error)}`;
      return false;
    }
  }

  dismiss(id: number) {
    this.notified = this.notified.map((n) => (n.id === id ? { ...n, card: false } : n));
  }

  /** Asks for Settings to open on the Accounts page, where the viewer signs in. */
  openAccounts() {
    this.accountsRequests++;
    this.signingIn = true;
  }

  /** Settings closed: if it was opened to sign in, the comments are synced again. */
  afterSettings() {
    if (!this.signingIn) return;
    this.signingIn = false;
    void this.sync();
  }

  private notify(notification: Notification) {
    const id = ++this.lastId;
    this.notified = [{ id, notification, at: Math.floor(Date.now() / 1000), card: true }, ...this.notified];
    // A stopped sync stays on screen until the viewer deals with it; the rest is only told.
    if (notification.kind === "info") setTimeout(() => this.dismiss(id), CARD_MS);
  }
}

export const platform = new PlatformStore();
