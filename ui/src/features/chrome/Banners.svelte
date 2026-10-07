<script lang="ts">
  // The notices under the header that the viewer can dismiss: a degraded file watcher and the
  // dependency restores that failed.
  import { untrack } from "svelte";
  import type { RestoreFailure } from "../../generated/RestoreFailure";
  import type { WatchDegraded } from "../../generated/WatchDegraded";
  import Banner from "../../lib/ui/Banner.svelte";
  import { degradedMessage } from "./degradedMessage";
  import { limitedMessage } from "./limitedMessage";

  let { degraded, failures }: { degraded?: WatchDegraded; failures: RestoreFailure[] } = $props();

  let dismissedDegraded = $state<WatchDegraded>();
  // How many failures the viewer dismissed: a longer list is news again, and a shorter one
  // (a project got fixed) lowers it so a failure that comes back is shown.
  let dismissedFailures = $state(0);
  $effect(() => {
    if (failures.length < untrack(() => dismissedFailures)) dismissedFailures = failures.length;
  });
</script>

{#if degraded && dismissedDegraded !== degraded}
  <Banner message={degradedMessage[degraded]} ondismiss={() => (dismissedDegraded = degraded)} />
{/if}

{#if failures.length > dismissedFailures}
  <Banner message={limitedMessage(failures)} ondismiss={() => (dismissedFailures = failures.length)}>
    {#snippet details()}
      <ul class="failures">
        {#each failures as failure (failure.project + failure.tool + failure.message)}
          <li>
            <strong>{failure.project}</strong> ({failure.tool}):
            <pre>{failure.message}</pre>
          </li>
        {/each}
      </ul>
    {/snippet}
  </Banner>
{/if}

<style>
  .failures {
    margin: 0;
    padding-left: 16px;
  }
  .failures pre {
    margin: 2px 0 6px;
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
  }
</style>
