<script lang="ts">
  // The notices under the header: HEAD off the review's base, which stays while it is so, and
  // those the viewer can dismiss: a degraded file watcher, the dependency restores that failed,
  // and a pull request whose analysis is limited until trusted.
  import { untrack } from "svelte";
  import type { PullRequestTrust } from "../../generated/PullRequestTrust";
  import type { RestoreFailure } from "../../generated/RestoreFailure";
  import type { WatchDegraded } from "../../generated/WatchDegraded";
  import Banner from "../../lib/ui/Banner.svelte";
  import { degradedMessage } from "./degradedMessage";
  import { limitedMessage } from "./limitedMessage";
  import RestoreFailureList from "./RestoreFailureList.svelte";
  import { offBaseMessage } from "./scopeText";
  import { trustMessage } from "./trustMessage";

  let {
    degraded,
    failures,
    offBase,
    pullRequest,
    ontrust,
  }: {
    degraded?: WatchDegraded;
    failures: RestoreFailure[];
    /** The short hash of the base the working tree's HEAD no longer builds on. */
    offBase?: string;
    /** The pull request on view, if one is. */
    pullRequest?: PullRequestTrust;
    /** Trusts the pull request on view at its current head. */
    ontrust: () => void;
  } = $props();

  let dismissedDegraded = $state<WatchDegraded>();
  // Dismissed until the pull request is trusted; a push that voids the trust is news again.
  let dismissedLimited = $state(false);
  $effect(() => {
    if (pullRequest?.trusted !== false) dismissedLimited = false;
  });
  // How many failures the viewer dismissed: a longer list is news again, and a shorter one
  // (a project got fixed) lowers it so a failure that comes back is shown.
  let dismissedFailures = $state(0);
  $effect(() => {
    if (failures.length < untrack(() => dismissedFailures)) dismissedFailures = failures.length;
  });
</script>

{#if offBase !== undefined}
  <Banner message={offBaseMessage(offBase)} />
{/if}

{#if degraded && dismissedDegraded !== degraded}
  <Banner message={degradedMessage[degraded]} ondismiss={() => (dismissedDegraded = degraded)} />
{/if}

{#if pullRequest && !pullRequest.trusted && !dismissedLimited}
  <Banner
    message={trustMessage(pullRequest)}
    ondismiss={() => (dismissedLimited = true)}
    action={{ label: "Trust this PR", onclick: ontrust }}
  />
{/if}

{#if failures.length > dismissedFailures}
  <Banner message={limitedMessage(failures)} ondismiss={() => (dismissedFailures = failures.length)}>
    {#snippet details()}
      <RestoreFailureList {failures} />
    {/snippet}
  </Banner>
{/if}
