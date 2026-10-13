import type { PullRequestTrust } from "../../generated/PullRequestTrust";

/** Says why the analysis of a pull request that is not trusted is limited, and what that costs. */
export function trustMessage(pullRequest: Pick<PullRequestTrust, "fork">): string {
  const why = pullRequest.fork ? "comes from a fork" : "is in a project that is not trusted";
  return `Analysis limited: this pull request ${why}, so dependencies are fetched without running install scripts and language servers that run project code are not started.`;
}
