<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import StatusDot from "../../lib/ui/StatusDot.svelte";
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

<span class="scope" class:narrowed={chip.narrowed} data-escapes-scope>
  {#if onopen}
    <button class="chip" type="button" title={chip.tip} onclick={onopen}>{chip.label}{@render dot()}</button>
  {:else}
    <span class="chip static" title={chip.tip}>{chip.label}{@render dot()}</span>
  {/if}
  {#if chip.narrowed && onclear}
    <button class="x" type="button" aria-label="Back to all changes" title="Back to all changes (Esc)" onclick={onclear}
      ><Icon name="close" size={12} stroke={1.8} /></button
    >
  {/if}
</span>

<style>
  .scope {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 0 2px;
    border: 1px solid transparent;
    border-radius: 7px;
  }
  .scope.narrowed {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--accent) 38%, transparent);
    color: var(--accent);
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
  .chip.static {
    color: var(--text-2);
  }
  .chip :global(.dot) {
    margin-left: 6px;
  }
  .x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--accent);
    cursor: default;
  }
  .x:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
</style>
