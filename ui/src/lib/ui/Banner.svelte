<script lang="ts">
  import type { Snippet } from "svelte";
  import IconButton from "./IconButton.svelte";

  let {
    message,
    ondismiss,
    details,
    action,
    tone = "warn",
  }: {
    message: string;
    /** What it tints with: amber for a caution, red for something that failed. */
    tone?: "warn" | "bad";
    /** Without it the banner stays while what it says holds. */
    ondismiss?: () => void;
    details?: Snippet;
    /** A button next to the message. */
    action?: { label: string; onclick: () => void };
  } = $props();

  let expanded = $state(false);
</script>

<div class="banner" role="status" style:--tone={tone === "bad" ? "var(--del-text)" : undefined}>
  <div class="line">
    <span class="selectable">{message}</span>
    {#if action}
      <button class="flat" type="button" onclick={action.onclick}>{action.label}</button>
    {/if}
    {#if details}
      <button class="flat" type="button" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
        {expanded ? "Hide details" : "Details"}
      </button>
    {/if}
    {#if ondismiss}
      <div class="dismiss">
        <IconButton icon="close" label="Dismiss" size={20} iconSize={12} onclick={ondismiss} />
      </div>
    {/if}
  </div>
  {#if details && expanded}
    <div class="details selectable">{@render details()}</div>
  {/if}
</div>

<style>
  .banner {
    --tone: var(--old-base);
    flex: none;
    padding: 4px 6px 4px 12px;
    background: color-mix(in srgb, var(--tone) 14%, var(--bg-panel));
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-ctl);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  span {
    margin-right: auto;
    padding-right: 10px;
  }
  /* The actions are plain text until pointed at; the hover and pressed boxes are the banner's own
     tone, a little stronger than its background. */
  button.flat {
    height: 20px;
    box-sizing: border-box;
    padding: 0 8px;
    border-radius: var(--radius);
    color: var(--text-2);
    line-height: 1;
  }
  button.flat:enabled:hover {
    background: color-mix(in srgb, var(--tone) 26%, var(--bg-panel));
  }
  button.flat:enabled:active {
    background: color-mix(in srgb, var(--tone) 38%, var(--bg-panel));
  }
  .dismiss {
    display: flex;
    margin-left: 4px;
  }
  .details {
    max-height: 30vh;
    overflow: auto;
    padding: 4px 0 4px 0;
    color: var(--text-2);
  }
</style>
