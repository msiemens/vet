// What the platform says of the pull request a session is of: its title, state, counts, the viewer's
// vote and the platform's traits. Only a session for a pull request has one.

import type { PullRequestInfo } from "../generated/PullRequestInfo";
import type { Backend } from "./backend";
import { Resource } from "./resource";

/** What the pull request info asks of its session. */
export interface PullRequestHost {
  backend(): Backend;
}

class PullRequest {
  /** Absent before the first answer, and when the platform could not give it. */
  info = $state.raw<PullRequestInfo>();
  private reads: Resource<PullRequestInfo>;
  private host: PullRequestHost;

  constructor(host: PullRequestHost) {
    this.host = host;
    this.reads = new Resource<PullRequestInfo>({
      apply: (info) => (this.info = info),
      failed: () => (this.info = undefined),
    });
  }

  /** Asks again what the platform said; the latest answer counts. */
  async reload() {
    await this.reads.load(() => this.host.backend().pullRequestInfo());
  }

  /** What the viewer has not read of the pull request as a whole. */
  get unreadCount(): number {
    return this.info?.unread.all ?? 0;
  }

  /** The viewer's comments and staged vote that wait to be posted. */
  get outgoingCount(): number {
    return this.info?.outgoing.count ?? 0;
  }

  /** The session ended: an answer still on its way counts for nothing. */
  clear() {
    this.reads.cancel();
    this.info = undefined;
  }
}

export type SessionPullRequest = PullRequest;

export const newPullRequest = (host: PullRequestHost): SessionPullRequest => new PullRequest(host);
