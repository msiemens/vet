<script lang="ts">
  import StatusDot from "../../lib/ui/StatusDot.svelte";
  import TagChip from "../../lib/ui/TagChip.svelte";
  import type { ScopeChip } from "./scopeText";

  let {
    chip,
    onopen,
    onclear,
  }: {
    chip: ScopeChip;
    /** Opens or focuses the timeline; without it (no versions) the chip is only text. */
    onopen?: () => void;
    /** Goes back to all changes; offered while the chip is narrowed. */
    onclear?: () => void;
  } = $props();
</script>

{#snippet dot()}
  {#if chip.dot}
    <StatusDot
      tone="ok"
      hollow={chip.dot === "pending"}
      label={chip.dot === "pending" ? "Edits wait to become a version" : "Follows the working tree"}
    />
  {/if}
{/snippet}

{#snippet label()}
  {#if onopen}
    <button class="chip" class:inside={chip.narrowed} type="button" title={chip.tip} onclick={onopen}
      >{chip.label}{@render dot()}</button
    >
  {:else}
    <span class="chip static" class:inside={chip.narrowed} title={chip.tip}>{chip.label}{@render dot()}</span>
  {/if}
{/snippet}

<span class="scope" data-escapes-scope>
  {#if chip.narrowed}
    <TagChip removeLabel="Back to all changes" removeTitle="Back to all changes (Esc)" onremove={onclear}>
      {@render label()}
    </TagChip>
  {:else}
    {@render label()}
  {/if}
</span>

<style>
  .scope {
    display: inline-flex;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    height: 24px;
    padding: 0 7px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: inherit;
    font-size: var(--fs-ctl);
    cursor: default;
  }
  button.chip:hover {
    background: var(--hover);
  }
  button.chip:active {
    background: var(--press);
  }
  .chip.inside {
    height: 16px;
    padding: 0 4px;
  }
  .chip.static {
    color: var(--text-2);
  }
  .chip :global(.dot) {
    margin-left: 6px;
  }
</style>
