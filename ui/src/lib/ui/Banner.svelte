<script lang="ts">
  import type { Snippet } from "svelte";
  import IconButton from "./IconButton.svelte";

  let {
    message,
    ondismiss,
    details,
    action,
  }: {
    message: string;
    ondismiss: () => void;
    details?: Snippet;
    /** A button next to the message. */
    action?: { label: string; onclick: () => void };
  } = $props();

  let expanded = $state(false);
</script>

<div class="banner" role="status">
  <div class="line">
    <span>{message}</span>
    {#if action}
      <button class="flat" type="button" onclick={action.onclick}>{action.label}</button>
    {/if}
    {#if details}
      <button class="flat" type="button" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
        {expanded ? "Hide details" : "Details"}
      </button>
    {/if}
    <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={ondismiss} />
  </div>
  {#if details && expanded}
    <div class="details">{@render details()}</div>
  {/if}
</div>

<style>
  .banner {
    flex: none;
    padding: 4px 6px 4px 12px;
    background: color-mix(in srgb, var(--old-base) 14%, var(--bg-panel));
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  span {
    flex: 1;
  }
  button {
    padding: 0 6px;
    border-radius: var(--radius);
    color: var(--text-2);
  }
  .details {
    max-height: 30vh;
    overflow: auto;
    padding: 4px 0 4px 0;
    color: var(--text-2);
  }
</style>
