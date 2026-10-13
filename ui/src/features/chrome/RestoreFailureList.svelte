<script lang="ts">
  // What a Banner shows under "Details" of the restores that failed: each project, its tool and its message.
  import type { RestoreFailure } from "../../generated/RestoreFailure";

  let { failures }: { failures: RestoreFailure[] } = $props();
</script>

<ul>
  {#each failures as failure (failure.project + failure.tool + failure.message)}
    <li>
      <strong>{failure.project}</strong> ({failure.tool}):
      <pre>{failure.message}</pre>
    </li>
  {/each}
</ul>

<style>
  ul {
    margin: 0;
    padding-left: 16px;
  }
  pre {
    margin: 2px 0 6px;
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
  }
</style>
