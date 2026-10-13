<script lang="ts">
  import type { CiState } from "../../generated/CiState";
  import { ciIcon } from "../ciStatus";
  import Icon from "./Icon.svelte";

  /** The icon of a commit's checks, coloured by how they went; nothing when there is none to show. */
  let { state, size = 13, title }: { state: CiState | undefined; size?: number; title?: string } = $props();
  const look = $derived(ciIcon(state));
</script>

{#if look}
  <span class="ci {look.tone}" title={title ?? look.title}><Icon name={look.icon} {size} stroke={2} /></span>
{/if}

<style>
  .ci {
    display: inline-flex;
  }
  .ok {
    color: var(--add-text);
  }
  .failing {
    color: var(--del-text);
  }
  .running {
    color: var(--old-base);
  }
  .waiting {
    color: var(--text-3);
  }
</style>
