<script lang="ts">
  import { onMount } from "svelte";
  import { formatBytes } from "../../lib/formatBytes";
  import Facts from "../../lib/ui/Facts.svelte";
  import { removeProject } from "../../state/removalPrompt.svelte";
  import { storage } from "../../state/storage.svelte";

  let { root }: { root: string } = $props();

  // The dialog mounts the page again for another project.
  onMount(() => storage.watch({ root }));
</script>

<div class="page">
  {#if storage.loadError}
    <p class="settings-problem">{storage.loadError}</p>
  {:else if !storage.project}
    <p class="empty">Measuring disk usage…</p>
  {:else}
    <Facts>
      <dt>Review data</dt>
      <dd>{formatBytes(storage.project.reviewData)}</dd>
      <dt>Source trees</dt>
      <dd>
        {formatBytes(storage.project.sourceTrees)}
        <span class="muted">{formatBytes(storage.project.sourceTreesInUse)} in use</span>
      </dd>
      <dt>Semantic cache</dt>
      <dd>{formatBytes(storage.project.semanticCache)}</dd>
      <dt></dt>
      <dd class="buttons">
        <button class="ctl" type="button" disabled={storage.busy === root} onclick={() => void storage.clear(root)}
          >Clear caches</button
        >
        <button class="ctl" type="button" onclick={() => void removeProject(root)}>Remove project…</button>
      </dd>
    </Facts>
  {/if}
</div>

<style>
  .page {
    font-size: var(--fs-ctl);
  }
  p {
    margin: 0;
  }
  .empty {
    color: var(--text-3);
  }
  .muted {
    color: var(--text-2);
  }
  .buttons {
    display: flex;
    gap: 6px;
  }
</style>
