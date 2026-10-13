<script lang="ts">
  import type { CiState } from "../../generated/CiState";
  import type { CheckPhase } from "../../generated/CheckPhase";
  import { ciIcon, phaseLook } from "../ciStatus";
  import Icon from "./Icon.svelte";

  /**
   * The icon of a commit's checks, coloured by how they went; nothing when there is none to show. With a
   * phase it is the icon of one run, job or step, which also shows what waits and what was stopped.
   */
  let {
    state,
    phase,
    size = 13,
    title,
  }: { state: CiState | undefined; phase?: CheckPhase; size?: number; title?: string } = $props();
  const look = $derived(phase !== undefined && state !== undefined ? phaseLook(state, phase) : ciIcon(state));
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
